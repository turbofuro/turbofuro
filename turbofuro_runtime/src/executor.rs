use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use std::vec;

use arc_swap::ArcSwap;
use async_recursion::async_recursion;

use nanoid::nanoid;
use serde;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
use tel::describe;
use tel::Description;
use tel::ObjectBody;
use tel::Selector;
use tel::SelectorPart;
use tel::StorageValue;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::SendTimeoutError;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::time::Instant;
use tracing::info;
use tracing::warn;
use tracing::{debug, instrument};

use crate::actions::actors;
use crate::actions::alarms;
use crate::actions::convert;
use crate::actions::crypto;
use crate::actions::debug;
use crate::actions::form_data;
use crate::actions::fs;
use crate::actions::http_client;
use crate::actions::http_server;
use crate::actions::kv;
use crate::actions::mail;
use crate::actions::mustache;
use crate::actions::os;
use crate::actions::postgres;
use crate::actions::pubsub;
use crate::actions::redis;
use crate::actions::tasks;
use crate::actions::time;
use crate::actions::wasm;
use crate::actions::websocket;
use crate::debug::DebugMessage;
use crate::debug::ExecutionLoggerHandle;
use crate::debug::LoggerMessage;
use crate::errors::ErrorRepresentation;
use crate::errors::ExecutionError;
use crate::evaluations::eval;
use crate::evaluations::eval_selector;
use crate::resources::{ActorResources, ResourceRegistry};

pub static VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ParameterDefinition {
    pub name: String,
    pub description: Option<String>,
}

fn default_exported() -> bool {
    false
}

#[derive(Debug, Clone)]
pub enum Callee {
    Local {
        function_id: String,
    },
    Import {
        import_name: String,
        function_id: String,
    },
}

impl serde::Serialize for Callee {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Callee::Local { function_id } => {
                serializer.serialize_str(&format!("local/{}", function_id))
            }
            Callee::Import {
                import_name,
                function_id,
            } => serializer.serialize_str(&format!("import/{}/{}", import_name, function_id)),
        }
    }
}

struct CalleeVisitor;

impl<'de> serde::de::Visitor<'de> for CalleeVisitor {
    type Value = Callee;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a string with format 'local/ID' or 'import/MODULE_ID:ID'")
    }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
        let parts: Vec<&str> = value.split('/').collect();

        match parts.as_slice() {
            ["local", function_id] => Ok(Callee::Local {
                function_id: function_id.to_string(),
            }),
            ["import", module_version_id, function_id] => Ok(Callee::Import {
                import_name: module_version_id.to_string(),
                function_id: function_id.to_string(),
            }),
            _ => Err(serde::de::Error::custom("Invalid format for Callee")),
        }
    }
}

impl<'de> serde::Deserialize<'de> for Callee {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(CalleeVisitor)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Branch {
    pub condition: String,
    pub steps: Steps,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum Step {
    Call {
        id: String,
        callee: Callee,
        parameters: Vec<Parameter>,
        #[serde(rename = "storeAs")]
        store_as: Option<String>,
    },
    DefineFunction {
        id: String,
        parameters: Vec<ParameterDefinition>,
        #[serde(default = "default_exported")]
        exported: bool,
        body: Steps,
        name: String,
    },
    DefineNativeFunction {
        id: String,
        name: String,
        #[serde(rename = "nativeId")]
        native_id: String,
        parameters: Vec<ParameterDefinition>,
        #[serde(default = "default_exported")]
        exported: bool,
    },
    If {
        id: String,
        condition: String,
        then: Steps,
        branches: Option<Vec<Branch>>,
        #[serde(rename = "else")]
        else_: Option<Steps>,
    },
    ForEach {
        id: String,
        items: String,
        item: String,
        body: Steps,
    },
    While {
        id: String,
        condition: String,
        body: Steps,
    },
    Return {
        id: String,
        value: Option<String>,
    },
    Break {
        id: String,
    },
    Continue {
        id: String,
    },
    Assign {
        id: String,
        value: String,
        to: String,
    },
    Try {
        id: String,
        body: Steps,
        catch: Steps,
    },
    Throw {
        id: String,
        code: String,
        message: String,
        details: Option<String>,
        metadata: Option<String>,
    },
}

pub type Steps = Vec<Step>;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum Parameter {
    Tel { name: String, expression: String },
    FunctionRef { name: String, id: String },
}

impl Parameter {
    pub fn tel(name: &str, expression: &str) -> Parameter {
        Parameter::Tel {
            name: name.to_owned(),
            expression: expression.to_owned(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Environment {
    pub id: String,
    pub variables: ObjectBody,
    pub secrets: ObjectBody,
}

impl Environment {
    pub fn new(id: String) -> Self {
        Environment {
            id,
            variables: HashMap::new(),
            secrets: HashMap::new(),
        }
    }

    pub fn new_empty() -> Self {
        Environment {
            id: "empty".to_owned(),
            variables: HashMap::new(),
            secrets: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.id == "empty"
    }
}

impl Default for Environment {
    fn default() -> Self {
        Self::new_empty()
    }
}

impl Step {
    pub fn get_step_id(&self) -> &str {
        match self {
            Step::Call { id, .. } => id,
            Step::If { id, .. } => id,
            Step::ForEach { id, .. } => id,
            Step::While { id, .. } => id,
            Step::DefineFunction { id, .. } => id,
            Step::Return { id, .. } => id,
            Step::Break { id, .. } => id,
            Step::Continue { id, .. } => id,
            Step::Assign { id, .. } => id,
            Step::DefineNativeFunction { id, .. } => id,
            Step::Try { id, .. } => id,
            Step::Throw { id, .. } => id,
        }
    }
}

#[derive(Debug)]
pub struct ExecutionTest {
    pub environment: Arc<Environment>,
    pub functions: HashMap<String, Steps>,
    pub resources: ActorResources,
    pub module: Arc<CompiledModule>,
    pub global: Arc<Global>,
    pub actor_id: String,
}

pub fn create_console_logger() -> ExecutionLoggerHandle {
    let (sender, mut receiver) = mpsc::channel::<LoggerMessage>(16);
    tokio::spawn(async move {
        while let Some(log) = receiver.recv().await {
            match log {
                LoggerMessage::Log(log) => {
                    info!("Execution log: {}", serde_json::to_string(&log).unwrap());
                }
            }
        }
    });
    sender
}

impl Default for ExecutionTest {
    fn default() -> Self {
        let id: String = nanoid!();

        ExecutionTest {
            environment: Arc::new(Environment::new("test_env".to_owned())),
            functions: HashMap::new(),
            resources: ActorResources::default(),
            module: Arc::new(CompiledModule {
                id,
                local_functions: vec![],
                exported_functions: vec![],
                handlers: HashMap::new(),
                imports: HashMap::new(),
                module_id: nanoid!(),
            }),
            global: Arc::new(GlobalBuilder::new().build()),
            actor_id: nanoid!(),
        }
    }
}

impl ExecutionTest {
    pub fn get_context(&mut self) -> ExecutionContext {
        let initial_storage = ObjectBody::new();

        ExecutionContext {
            id: "test_id".to_owned(),
            actor_id: self.actor_id.clone(),
            log: ExecutionLog::new_started(
                describe(StorageValue::Object(initial_storage.clone())),
                "test",
                "Test function",
            ),
            storage: initial_storage,
            environment: self.environment.clone(),
            resources: &mut self.resources,
            mode: ExecutionMode::Probe,
            loop_counts: vec![],
            bubbling: false,
            references: HashMap::new(),
            module: self.module.clone(),
            global: self.global.clone(),
        }
    }

    pub fn get_debug_context(&mut self, debugger_handle: DebuggerHandle) -> ExecutionContext {
        let initial_storage = ObjectBody::new();

        ExecutionContext {
            id: "test_id".to_owned(),
            actor_id: self.actor_id.clone(),
            log: ExecutionLog::new_started(
                describe(StorageValue::Object(initial_storage.clone())),
                "test",
                "Test function",
            ),
            storage: initial_storage,
            environment: self.environment.clone(),
            resources: &mut self.resources,
            mode: ExecutionMode::Debug(debugger_handle),
            loop_counts: vec![],
            bubbling: false,
            references: HashMap::new(),
            module: self.module.clone(),
            global: self.global.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Function {
    Normal {
        id: String,
        body: Steps,
        name: String,
    },
    Native {
        id: String,
        native_id: String,
        name: String,
    },
}

impl Function {
    pub fn get_id(&self) -> &str {
        match self {
            Function::Normal { id, .. } => id,
            Function::Native { id, .. } => id,
        }
    }

    pub fn get_name(&self) -> &str {
        match self {
            Function::Normal { name, .. } => name,
            Function::Native { name, .. } => name,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Import {
    #[serde(rename_all = "camelCase")]
    Cloud { id: String, version_id: String },
}

#[derive(Debug, Clone)]
pub struct CompiledModule {
    pub id: String, // Module version id
    pub module_id: String,
    pub local_functions: Vec<Function>,
    pub exported_functions: Vec<Function>,
    pub handlers: HashMap<String, String>,
    pub imports: HashMap<String, Arc<CompiledModule>>,
}

impl CompiledModule {
    pub fn get_function(&self, function_id: &str) -> Result<&Function, ExecutionError> {
        self.local_functions
            .iter()
            .find(|f| f.get_id() == function_id)
            .or_else(|| {
                self.exported_functions
                    .iter()
                    .find(|f| f.get_id() == function_id)
            })
            .ok_or(ExecutionError::FunctionNotFound {
                id: function_id.to_owned(),
            })
    }
}

#[derive(Debug)]
pub struct Global {
    pub modules: RwLock<Vec<Arc<CompiledModule>>>,
    pub registry: ResourceRegistry,
    pub execution_logger: ExecutionLoggerHandle,
    pub environment: ArcSwap<Environment>,
    pub pub_sub: Mutex<HashMap<String, tokio::sync::broadcast::Sender<StorageValue>>>,
    pub debug_state: ArcSwap<DebugState>,
}

#[derive(Debug, Clone)]
pub struct DebugEntry {
    pub module_id: String,
    pub debugger_handle: DebuggerHandle,
    pub last_activity: Instant,
    pub module: Option<Arc<CompiledModule>>, // If specified, this module will be applied to the configuration
}

#[derive(Debug, Default, Clone)]
pub struct DebugState {
    pub entries: Vec<DebugEntry>,
}

impl DebugState {
    pub fn get_debugger(&self, module_id: &str) -> Option<DebuggerHandle> {
        self.entries
            .iter()
            .find(|e| e.module_id == module_id)
            .map(|e| e.debugger_handle.clone())
    }

    pub fn get_entry(&self, module_id: &str) -> Option<&DebugEntry> {
        self.entries.iter().find(|e| e.module_id == module_id)
    }

    pub fn add_or_update_entry(
        &mut self,
        module_id: String,
        debugger_handle: DebuggerHandle,
        module: Option<Arc<CompiledModule>>,
    ) {
        let entry = self.entries.iter_mut().find(|e| e.module_id == module_id);
        if let Some(entry) = entry {
            entry.debugger_handle = debugger_handle;
            entry.last_activity = Instant::now();
            entry.module = module;
        } else {
            self.entries.push(DebugEntry {
                module_id,
                debugger_handle,
                last_activity: Instant::now(),
                module,
            });
        }
    }

    pub fn remove_entry(&mut self, module_id: &str) -> Option<DebugEntry> {
        let entry = self.entries.iter_mut().find(|e| e.module_id == module_id);
        if let Some(entry) = entry {
            let entry = entry.clone();
            self.entries.retain(|e| e.module_id != module_id);
            Some(entry)
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub struct GlobalBuilder {
    modules: Vec<Arc<CompiledModule>>,
    registry: ResourceRegistry,
    execution_logger: ExecutionLoggerHandle,
    pub_sub: HashMap<String, tokio::sync::broadcast::Sender<StorageValue>>,
    environment: Environment,
}

impl Default for GlobalBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalBuilder {
    pub fn new() -> Self {
        GlobalBuilder {
            modules: vec![],
            registry: ResourceRegistry::default(),
            execution_logger: create_console_logger(),
            pub_sub: HashMap::new(),
            environment: Environment::default(),
        }
    }

    pub fn module(mut self, module: Arc<CompiledModule>) -> Self {
        self.modules.push(module);
        self
    }

    pub fn registry(mut self, registry: ResourceRegistry) -> Self {
        self.registry = registry;
        self
    }

    pub fn environment(mut self, environment: Environment) -> Self {
        self.environment = environment;
        self
    }

    pub fn execution_logger(mut self, execution_logger: ExecutionLoggerHandle) -> Self {
        self.execution_logger = execution_logger;
        self
    }

    pub fn pub_sub(
        mut self,
        key: String,
        sender: tokio::sync::broadcast::Sender<StorageValue>,
    ) -> Self {
        self.pub_sub.insert(key, sender);
        self
    }

    pub fn build(self) -> Global {
        Global {
            modules: RwLock::new(self.modules),
            registry: self.registry,
            execution_logger: self.execution_logger,
            pub_sub: self.pub_sub.into(),
            environment: ArcSwap::new(Arc::new(self.environment)),
            debug_state: ArcSwap::new(Arc::new(DebugState::default())),
        }
    }
}

pub fn get_timestamp() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(n) => n.as_millis().try_into().unwrap_or_default(),
        Err(_) => 0,
    }
}

#[derive(Debug, Clone)]
pub struct DebuggerHandle {
    pub sender: mpsc::Sender<DebugMessage>,
}

impl DebuggerHandle {
    pub fn new() -> (Self, mpsc::Receiver<DebugMessage>) {
        let (sender, receiver) = mpsc::channel::<DebugMessage>(16);
        (Self { sender }, receiver)
    }
}

#[derive(Debug, Clone)]
pub enum ExecutionMode {
    /// Optimize for speed with minimal observability
    Fast,
    /// Collect execution events for later analysis with acceptable performance overhead
    Probe,
    /// Debug mode with live observability with the cost of performance overhead
    Debug(DebuggerHandle),
}

#[derive(Debug)]
pub struct ExecutionContext<'a> {
    pub id: String,

    pub actor_id: String,
    pub log: ExecutionLog,
    pub storage: ObjectBody,
    pub references: HashMap<String, String>,

    /// Exception bubbling
    /// If set to true, the context is currently bubbling an exception
    pub bubbling: bool,

    pub module: Arc<CompiledModule>,
    pub global: Arc<Global>,

    pub resources: &'a mut ActorResources,
    pub environment: Arc<Environment>,

    pub mode: ExecutionMode,
    pub loop_counts: Vec<(String, u64)>,
}

// TODO: Figure out proper error handling for such cases
fn handle_logging_error(result: Result<(), SendTimeoutError<DebugMessage>>) {
    match result {
        Ok(_) => {
            // No-op
        }
        Err(e) => match e {
            SendTimeoutError::Timeout(_) => {
                warn!("Logging channel send timeout")
            }
            SendTimeoutError::Closed(_) => {
                warn!("Logging channel is closed")
            }
        },
    }
}

impl<'a> ExecutionContext<'a> {
    async fn report_verbose_event(&mut self, event: ExecutionEvent) {
        match &self.mode {
            ExecutionMode::Fast => {
                // No-op
            }
            ExecutionMode::Probe => {
                let lc = self.loop_counts.last().map(|(_id, c)| c);
                if let Some(lc) = lc {
                    if lc < &10 {
                        self.log.events.push(event);
                    }
                } else {
                    self.log.events.push(event);
                }
            }
            ExecutionMode::Debug(debugger) => {
                let lc = self.loop_counts.last().map(|(_id, c)| c);
                if let Some(lc) = lc {
                    // info!("Loop count: {}", lc);
                    if lc < &10 {
                        handle_logging_error(
                            debugger
                                .sender
                                .send_timeout(
                                    DebugMessage::AppendEventToReport {
                                        id: self.id.clone(),
                                        event: event.clone(),
                                    },
                                    std::time::Duration::from_secs(5),
                                )
                                .await,
                        );
                        self.log.events.push(event);
                    }
                } else {
                    handle_logging_error(
                        debugger
                            .sender
                            .send_timeout(
                                DebugMessage::AppendEventToReport {
                                    id: self.id.clone(),
                                    event: event.clone(),
                                },
                                std::time::Duration::from_secs(5),
                            )
                            .await,
                    );
                    self.log.events.push(event);
                }
            }
        }
    }

    pub async fn add_step_started(&mut self, id: &str) {
        self.report_verbose_event(ExecutionEvent::StepStarted {
            id: id.to_owned(),
            timestamp: get_timestamp(),
        })
        .await;
    }

    pub async fn add_step_finished(&mut self, id: &str) {
        self.report_verbose_event(ExecutionEvent::StepFinished {
            id: id.to_owned(),
            timestamp: get_timestamp(),
        })
        .await;
    }

    pub async fn add_enter_function(&mut self, function_id: String, initial_storage: ObjectBody) {
        self.report_verbose_event(ExecutionEvent::EnterFunction {
            function_id,
            initial_storage: describe(tel::StorageValue::Object(initial_storage)),
        })
        .await;
    }

    pub async fn add_leave_function(&mut self, function_id: String) {
        self.report_verbose_event(ExecutionEvent::LeaveFunction { function_id })
            .await;
    }

    pub async fn add_error_thrown(&mut self, id: &str, error: ExecutionError) {
        match &self.mode {
            ExecutionMode::Fast => {
                let event = ExecutionEvent::ErrorThrown {
                    id: id.to_owned(),
                    error: error.into(),
                    // Add snapshot so the user can see the state of the context when the error was thrown
                    snapshot: Some(ContextSnapshot {
                        storage: self.storage.clone(),
                        references: self.references.clone(),
                    }),
                };
                self.log.events.push(event);
            }
            ExecutionMode::Probe => {
                let event = ExecutionEvent::ErrorThrown {
                    id: id.to_owned(),
                    error: error.into(),
                    // No need to add snapshot in probe mode as the user can retrieve the context using events
                    snapshot: None,
                };
                self.log.events.push(event);
            }
            ExecutionMode::Debug(debugger) => {
                let event = ExecutionEvent::ErrorThrown {
                    id: id.to_owned(),
                    error: error.into(),
                    // No need to add snapshot in probe mode as the user can retrieve the context using events
                    snapshot: None,
                };
                handle_logging_error(
                    debugger
                        .sender
                        .send_timeout(
                            DebugMessage::AppendEventToReport {
                                id: self.id.clone(),
                                event: event.clone(),
                            },
                            std::time::Duration::from_secs(5),
                        )
                        .await,
                );
                self.log.events.push(event);
            }
        }
    }

    pub async fn start_report(&mut self) {
        if let ExecutionMode::Debug(debugger) = &self.mode {
            handle_logging_error(
                debugger
                    .sender
                    .send_timeout(
                        DebugMessage::StartReport {
                            id: self.id.clone(),
                            started_at: self.log.started_at,
                            initial_storage: self.log.initial_storage.clone(),
                            module_id: self.module.module_id.clone(),
                            module_version_id: self.module.id.clone(),
                            environment_id: self.environment.id.clone(),
                            function_id: self.log.function_id.clone(),
                            function_name: self.log.function_name.clone(),
                            status: ExecutionStatus::Started,
                            events: vec![],
                            metadata: None,
                        },
                        std::time::Duration::from_secs(5),
                    )
                    .await,
            );
        }
    }

    pub async fn end_report(&mut self, final_status: ExecutionStatus) {
        let now = get_timestamp();
        self.log.status = final_status;
        self.log.finished_at = Some(now);

        if let ExecutionMode::Debug(debugger) = &self.mode {
            handle_logging_error(
                debugger
                    .sender
                    .send_timeout(
                        DebugMessage::EndReport {
                            id: self.id.clone(),
                            status: self.log.status.clone(),
                            finished_at: now,
                        },
                        std::time::Duration::from_secs(5),
                    )
                    .await,
            );
        }
    }

    pub async fn add_to_storage(
        &mut self,
        id: &str,
        selector: Vec<SelectorPart>,
        value: StorageValue,
    ) -> Result<(), ExecutionError> {
        tel::store_value(&selector, &mut self.storage, value.clone())
            .map_err(ExecutionError::from)?;

        self.report_verbose_event(ExecutionEvent::StorageUpdated {
            id: id.to_string(),
            selector,
            value: describe(value),
        })
        .await;

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionStatus {
    // Execution finished successfully
    Finished,
    /// Execution was stopped unexpectedly by an error
    Failed,
    /// Execution has been started but not finished yet
    Started,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionLog {
    pub status: ExecutionStatus,
    pub function_id: String,
    pub function_name: String,
    pub initial_storage: Description,
    pub events: Vec<ExecutionEvent>,
    pub started_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Result<StorageValue, ExecutionError>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ExecutionReportMetadata {
    Http {
        path: String,
        method: String,
        status: u16,
    },
    Redis {
        command: String,
    },
    Alarm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionReport {
    pub status: ExecutionStatus,
    pub function_id: String,
    pub function_name: String,
    pub initial_storage: Description,
    pub events: Vec<ExecutionEvent>,
    pub module_id: String,
    pub module_version_id: String,
    pub environment_id: String,
    pub started_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextSnapshot {
    pub storage: ObjectBody,
    pub references: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", tag = "type")]
pub enum ExecutionEvent {
    StepStarted {
        id: String,
        timestamp: u64,
    },
    StepFinished {
        id: String,
        timestamp: u64,
    },
    ErrorThrown {
        id: String,
        error: ErrorRepresentation,
        snapshot: Option<ContextSnapshot>,
    },
    StorageUpdated {
        id: String,
        selector: Selector,
        value: Description,
    },
    #[serde(rename_all = "camelCase")]
    EnterFunction {
        function_id: String,
        initial_storage: Description,
    },
    #[serde(rename_all = "camelCase")]
    LeaveFunction {
        function_id: String,
    },
}

impl ExecutionLog {
    pub fn new_started(
        initial_storage: Description,
        function_id: &str,
        function_name: &str,
    ) -> Self {
        ExecutionLog {
            status: ExecutionStatus::Started,
            initial_storage,
            events: Vec::new(),
            started_at: get_timestamp(),
            finished_at: None,
            result: None,
            function_id: function_id.to_owned(),
            function_name: function_name.to_owned(),
        }
    }
}

async fn execute_native<'a>(
    native_id: &str,
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    store_as: Option<&str>,
    step_id: &str,
) -> Result<(), ExecutionError> {
    match native_id {
        "wasm/run_wasi" => wasm::run_wasi(context, parameters, step_id, store_as).await?,
        "fs/open" => fs::open_file(context, parameters, step_id, store_as).await?,
        // TODO: Before stable, decide how to structure the fs actions write_stream vs write_string, one is resourceful (requires open) and other is not
        "fs/write_stream" => fs::write_stream(context, step_id).await?,
        "fs/write_string" => fs::simple_write_string(context, parameters, step_id).await?,
        "fs/read_to_string" => {
            fs::simple_read_to_string(context, parameters, step_id, store_as).await?
        }
        "fs/setup_watcher" => fs::setup_watcher(context, parameters, step_id).await?,
        "fs/cancel_watcher" => fs::cancel_watcher(context, parameters, step_id).await?,
        "fs/read_dir" => fs::read_dir(context, parameters, step_id, store_as).await?,
        "alarms/set_alarm" => alarms::set_alarm(context, parameters, step_id).await?,
        "alarms/set_interval" => alarms::set_interval(context, parameters, step_id).await?,
        "alarms/cancel" => alarms::cancel_alarm(context, parameters, step_id).await?,
        "alarms/setup_cronjob" => alarms::setup_cronjob(context, parameters, step_id).await?,
        "time/sleep" => time::sleep(context, parameters, step_id).await?,
        "time/get_current_time" => {
            time::get_current_time(context, parameters, step_id, store_as).await?
        }
        "time/get_current_datetime" => {
            time::get_current_datetime(context, parameters, step_id, store_as).await?
        }
        "actors/spawn" => actors::spawn_actor(context, parameters, step_id, store_as).await?,
        "os/run_command" => os::run_command(context, parameters, step_id, store_as).await?,
        "os/read_environment_variable" => {
            os::read_environment_variable(context, parameters, step_id, store_as).await?
        }
        "os/set_environment_variable" => {
            os::set_environment_variable(context, parameters, step_id).await?
        }
        "actors/terminate" => actors::terminate(context, parameters, step_id).await?,
        "actors/send_command" => actors::send(context, parameters, step_id).await?, // TODO: Remove this once the new actors/send is fully implemented
        "actors/send" => actors::send(context, parameters, step_id).await?,
        "actors/request" => actors::request(context, parameters, step_id, store_as).await?,
        "actors/get_actor_id" => {
            actors::get_actor_id(context, parameters, step_id, store_as).await?
        }
        "actors/check_actor_exists" => {
            actors::check_actor_exists(context, parameters, step_id, store_as).await?
        }
        "http_client/request" => {
            http_client::send_http_request(context, parameters, step_id, store_as).await?
        }
        "http_client/request_with_stream" => {
            http_client::send_http_request_with_stream(context, parameters, step_id, store_as)
                .await?
        }
        "http_client/request_with_form_data" => {
            http_client::send_http_request_with_form_data(context, parameters, step_id, store_as)
                .await?
        }
        "http_client/stream_request" => {
            http_client::stream_http_request(context, parameters, step_id, store_as).await?
        }
        "http_client/stream_request_with_stream" => {
            http_client::stream_http_request_with_stream(context, parameters, step_id, store_as)
                .await?
        }
        "http_client/stream_request_with_form_data" => {
            http_client::stream_http_request_with_form_data(context, parameters, step_id, store_as)
                .await?
        }
        "http_client/build_client" => {
            http_client::build_client(context, parameters, step_id, store_as).await?
        }
        "form_data/create" => form_data::create_form_data(context, parameters, step_id, store_as)?,
        "form_data/add_stream_part" => {
            form_data::add_stream_part_to_form_data(context, parameters, step_id, store_as).await?
        }
        "form_data/add_text_part" => {
            form_data::add_text_part_to_form_data(context, parameters, step_id, store_as).await?
        }
        "http_server/setup_route" => http_server::setup_route(context, parameters, step_id).await?,
        "http_server/setup_streaming_route" => {
            http_server::setup_streaming_route(context, parameters, step_id).await?
        }
        "http_server/respond_with" => {
            http_server::respond_with(context, parameters, step_id).await?
        }
        "http_server/respond_with_stream" => {
            http_server::respond_with_stream(context, parameters, step_id).await?
        }
        "http_server/respond_with_sse_stream" => {
            http_server::respond_with_sse_stream(context, parameters, step_id, store_as).await?
        }
        "http_server/send_sse" => {
            http_server::send_sse(context, parameters, step_id, store_as).await?
        }
        "http_server/close_sse_stream" => {
            http_server::close_sse_stream(context, parameters, step_id, store_as).await?
        }
        "postgres/get_connection" => postgres::get_connection(context, parameters, step_id).await?,
        "postgres/query_one" => postgres::query_one(context, parameters, step_id, store_as).await?,
        "postgres/query" => postgres::query(context, parameters, step_id, store_as).await?,
        "redis/low_level" => {
            redis::low_level_command(context, parameters, step_id, store_as).await?
        }
        "redis/get_connection" => redis::get_connection(context, parameters, step_id).await?,
        "redis/subscribe" => redis::subscribe(context, parameters, step_id).await?,
        "redis/unsubscribe" => redis::unsubscribe(context, parameters, step_id).await?,
        "websocket/accept_ws" => websocket::accept_ws(context, parameters, step_id).await?,
        "websocket/send_message" => websocket::send_message(context, parameters, step_id).await?,
        "websocket/close" => websocket::close_websocket(context, parameters, step_id).await?,
        "kv/write" => kv::write_to_store(context, parameters, step_id).await?,
        "kv/read" => kv::read_from_store(context, parameters, step_id, store_as).await?,
        "kv/delete" => kv::delete_from_store(context, parameters, step_id).await?,
        "kv/increment" => kv::increment_store(context, parameters, step_id).await?,
        "convert/parse_json" => convert::parse_json(context, parameters, step_id, store_as).await?,
        "convert/to_json" => convert::to_json(context, parameters, step_id, store_as).await?,
        "convert/parse_urlencoded" => {
            convert::parse_urlencoded(context, parameters, step_id, store_as).await?
        }
        "convert/to_urlencoded" => {
            convert::to_urlencoded(context, parameters, step_id, store_as).await?
        }
        "convert/parse_url" => convert::parse_url(context, parameters, step_id, store_as).await?,
        "convert/to_url" => convert::to_url(context, parameters, step_id, store_as).await?,
        "crypto/get_uuid_v4" => crypto::get_uuid_v4(context, parameters, step_id, store_as).await?,
        "crypto/get_uuid_v7" => crypto::get_uuid_v7(context, parameters, step_id, store_as).await?,
        "crypto/jwt_decode" => crypto::jwt_decode(context, parameters, step_id, store_as).await?,
        "pubsub/publish" => pubsub::publish(context, parameters, step_id).await?,
        "pubsub/subscribe" => pubsub::subscribe(context, parameters, step_id).await?,
        "pubsub/unsubscribe" => pubsub::unsubscribe(context, parameters, step_id).await?,
        "mustache/render_template" => {
            mustache::render_template(context, parameters, step_id, store_as).await?
        }
        "mail/send_smtp_html" => {
            mail::sendmail_smtp_html(context, parameters, step_id, store_as).await?
        }
        "mail/send_smtp_text" => {
            mail::sendmail_smtp_text(context, parameters, step_id, store_as).await?
        }
        "tasks/run_task_continuously" => {
            tasks::run_task_continuously(context, parameters, step_id).await?
        }
        "tasks/cancel_task" => tasks::cancel_task(context, parameters, step_id).await?,
        "debug/ask_for_input" => {
            debug::ask_for_input(context, parameters, step_id, store_as).await?
        }
        "debug/show_result" => debug::show_result(context, parameters, step_id, store_as).await?,
        "debug/show_notification" => {
            debug::show_notification(context, parameters, step_id, store_as).await?
        }
        "debug/play_sound" => debug::play_sound(context, parameters, step_id, store_as).await?,
        id => {
            return Err(ExecutionError::Unsupported {
                message: format!("Native function {} not found", id),
            });
        }
    }
    Ok(())
}

pub fn evaluate_parameters(
    parameters: &[Parameter],
    storage: &ObjectBody,
    environment: &Environment,
) -> Result<(ObjectBody, HashMap<String, String>), ExecutionError> {
    let mut initial_storage = HashMap::new();
    let mut initial_references = HashMap::new();

    for parameter in parameters.iter() {
        match parameter {
            Parameter::Tel { name, expression } => {
                let value = eval(expression, storage, environment)?;
                initial_storage.insert(name.clone(), value);
            }
            Parameter::FunctionRef { name, id } => {
                initial_references.insert(name.clone(), id.clone());
            }
        }
    }

    Ok((initial_storage, initial_references))
}

async fn execute_function<'a>(
    module: Arc<CompiledModule>,
    function_id: &str,
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    store_as: Option<&str>,
    step_id: &str,
) -> Result<(), ExecutionError> {
    let function = module.get_function(function_id)?;
    match function {
        Function::Normal { id: _, body, name } => {
            let (initial_storage, initial_references) =
                evaluate_parameters(parameters, &context.storage, &context.environment)?;

            context
                .add_enter_function(function_id.to_owned(), initial_storage.clone())
                .await;

            let mut function_context = ExecutionContext {
                id: context.id.clone(),
                actor_id: context.actor_id.clone(),
                log: ExecutionLog::new_started(
                    describe(StorageValue::Object(initial_storage.clone())),
                    function_id,
                    name,
                ),
                storage: initial_storage,
                environment: context.environment.clone(),
                resources: context.resources,
                global: context.global.clone(),
                module: module.clone(),
                bubbling: false,
                references: initial_references,
                mode: context.mode.clone(),
                loop_counts: vec![],
            };

            let returned_value = match execute_steps(body, &mut function_context).await {
                Ok(_) => StorageValue::Null(None),
                Err(e) => match e {
                    ExecutionError::Return { value } => value,
                    e => {
                        context.log.events.append(&mut function_context.log.events);
                        context.add_leave_function(function_id.to_owned()).await;
                        return Err(e);
                    }
                },
            };

            context.log.events.append(&mut function_context.log.events);
            context.add_leave_function(function_id.to_owned()).await;

            if let Some(expression) = store_as {
                let selector = eval_selector(expression, &context.storage, &context.environment)?;
                context
                    .add_to_storage(step_id, selector, returned_value)
                    .await?;
                return Ok(());
            }
            Ok(())
        }
        Function::Native {
            id: _,
            native_id,
            name: _,
        } => match execute_native(native_id, context, parameters, store_as, step_id).await {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        },
    }
}

async fn for_each_inner<'a>(
    context: &mut ExecutionContext<'a>,
    step_id: &str,
    value: StorageValue,
    selector: Vec<SelectorPart>,
    body: &Steps,
) -> Result<(), ExecutionError> {
    match value {
        StorageValue::Array(arr) => {
            for item in arr {
                if let Some((_id, c)) = context.loop_counts.last_mut() {
                    *c += 1;
                }
                context
                    .add_to_storage(step_id, selector.clone(), item)
                    .await?;

                match execute_steps(body, context).await {
                    Ok(_) => {}
                    Err(e) => match e {
                        ExecutionError::Break => {
                            break;
                        }
                        ExecutionError::Continue => {
                            continue;
                        }
                        e => {
                            return Err(e);
                        }
                    },
                }
            }
        }
        StorageValue::Object(obj) => {
            for (key, value) in obj {
                if let Some((_id, c)) = context.loop_counts.last_mut() {
                    *c += 1;
                }
                context
                    .add_to_storage(
                        step_id,
                        selector.clone(),
                        StorageValue::Object(vec![(key.clone(), value)].into_iter().collect()),
                    )
                    .await?;

                match execute_steps(body, context).await {
                    Ok(_) => {}
                    Err(e) => match e {
                        ExecutionError::Break => {
                            break;
                        }
                        ExecutionError::Continue => {
                            continue;
                        }
                        e => {
                            return Err(e);
                        }
                    },
                }
            }
        }
        StorageValue::String(s) => {
            for c in s.chars() {
                if let Some((_id, c)) = context.loop_counts.last_mut() {
                    *c += 1;
                }
                context
                    .add_to_storage(
                        step_id,
                        selector.clone(),
                        StorageValue::String(c.to_string()),
                    )
                    .await?;

                match execute_steps(body, context).await {
                    Ok(_) => {}
                    Err(e) => match e {
                        ExecutionError::Break => {
                            break;
                        }
                        ExecutionError::Continue => {
                            continue;
                        }
                        e => {
                            return Err(e);
                        }
                    },
                }
            }
        }
        v => {
            return Err(ExecutionError::ParameterTypeMismatch {
                name: "items".to_string(),
                expected: Description::Union {
                    of: vec![
                        Description::new_base_type("array"),
                        Description::new_base_type("string"),
                        Description::new_base_type("object"),
                    ],
                },
                actual: describe(v),
            });
        }
    };

    Ok(())
}

async fn while_inner<'a>(
    context: &mut ExecutionContext<'a>,
    condition: &str,
    body: &Steps,
) -> Result<(), ExecutionError> {
    while eval(condition, &context.storage, &context.environment)? == StorageValue::Boolean(true) {
        if let Some((_id, c)) = context.loop_counts.last_mut() {
            *c += 1;
        }
        match execute_steps(body, context).await {
            Ok(_) => {}
            Err(e) => match e {
                ExecutionError::Break => {
                    break;
                }
                ExecutionError::Continue => {
                    continue;
                }
                e => {
                    return Err(e);
                }
            },
        }
    }
    Ok(())
}

#[async_recursion]
async fn execute_step<'a>(
    step: &Step,
    context: &mut ExecutionContext<'a>,
    step_id: &str,
) -> Result<(), ExecutionError> {
    debug!("Step: {}", step_id);
    match step {
        Step::Break { .. } => {
            return Err(ExecutionError::Break);
        }
        Step::Continue { .. } => {
            return Err(ExecutionError::Continue);
        }
        Step::Return { id: _, value } => {
            let value = match value {
                Some(value) => eval(value, &context.storage, &context.environment)?,
                None => StorageValue::Null(None),
            };
            return Err(ExecutionError::Return { value });
        }
        Step::DefineFunction { .. } => {
            return Err(ExecutionError::Unsupported {
                message: "Can't define function at runtime".to_owned(),
            });
        }
        Step::DefineNativeFunction { .. } => {
            return Err(ExecutionError::Unsupported {
                message: "Can't define native function at runtime".to_owned(),
            });
        }
        Step::Assign { value, to, .. } => {
            let value = eval(value, &context.storage, &context.environment)?;
            let selector = eval_selector(to, &context.storage, &context.environment)?;

            context.add_to_storage(step_id, selector, value).await?;
        }
        Step::Call {
            id: _,
            callee,
            parameters,
            store_as,
        } => match callee {
            Callee::Local { function_id } => {
                execute_function(
                    context.module.clone(),
                    function_id,
                    context,
                    parameters,
                    store_as.as_deref(),
                    step_id,
                )
                .await?
            }
            Callee::Import {
                import_name,
                function_id,
            } => {
                let module = context.module.imports.get(import_name).cloned();

                if let Some(module) = module {
                    execute_function(
                        module,
                        function_id,
                        context,
                        parameters,
                        store_as.as_deref(),
                        step_id,
                    )
                    .await?
                } else {
                    return Err(ExecutionError::UnresolvedImport {
                        import_name: import_name.to_owned(),
                    });
                }
            }
        },
        Step::If {
            condition,
            then,
            id: _,
            branches,
            else_,
        } => {
            let value = eval(condition, &context.storage, &context.environment)?;
            if value == StorageValue::Boolean(true) {
                execute_steps(then, context).await?;
            } else {
                let mut matched = false;

                // Else-if branches
                if let Some(branches) = branches {
                    for branch in branches {
                        let value =
                            eval(&branch.condition, &context.storage, &context.environment)?;
                        if value == StorageValue::Boolean(true) {
                            execute_steps(&branch.steps, context).await?;
                            matched = true;
                            break;
                        }
                    }
                }

                // If nothing matched, execute else branch
                if !matched {
                    if let Some(else_) = else_ {
                        execute_steps(else_, context).await?;
                    }
                }
            }
        }
        Step::ForEach {
            id,
            items,
            item,
            body,
        } => {
            let selector = eval_selector(item, &context.storage, &context.environment)?;
            let value = eval(items, &context.storage, &context.environment)?;

            let current_lc = context.loop_counts.last().map(|(_id, c)| c).unwrap_or(&0);
            context.loop_counts.push((id.clone(), *current_lc / 4));

            let result = for_each_inner(context, step_id, value, selector, body).await;
            context.loop_counts.pop();
            result?
        }
        Step::While {
            id,
            condition,
            body,
        } => {
            let current_lc = context.loop_counts.last().map(|(_id, c)| c).unwrap_or(&0);
            context.loop_counts.push((id.clone(), *current_lc / 4));

            let result = while_inner(context, condition, body).await;
            context.loop_counts.pop();
            result?
        }
        Step::Try { id, body, catch } => match execute_steps(body, context).await {
            Ok(_) => {}
            Err(e) => match e {
                ExecutionError::Break => {
                    return Err(ExecutionError::Break);
                }
                ExecutionError::Continue => {
                    return Err(ExecutionError::Continue);
                }
                ExecutionError::Return { .. } => {
                    return Err(e);
                }
                e => {
                    let value = ErrorRepresentation::from(e).to_value();
                    context
                        .add_to_storage(
                            id,
                            vec![SelectorPart::Identifier("error".to_owned())],
                            value,
                        )
                        .await?;
                    execute_steps(catch, context).await?;
                }
            },
        },
        Step::Throw {
            id: _,
            code,
            message,
            details,
            metadata,
        } => {
            let error = ExecutionError::Custom {
                inner_code: code.to_owned(),
                message: message.to_owned(),
                details: {
                    let mut evaluated: Option<StorageValue> = None;
                    if let Some(value) = details {
                        evaluated = Some(eval(value, &context.storage, &context.environment)?);
                    }
                    evaluated
                },
                metadata: {
                    let mut evaluated: Option<StorageValue> = None;
                    if let Some(value) = metadata {
                        evaluated = Some(eval(value, &context.storage, &context.environment)?);
                    }
                    evaluated
                },
            };
            return Err(error);
        }
    }
    Ok(())
}

#[async_recursion]
async fn execute_steps<'a>(
    steps: &Steps,
    context: &mut ExecutionContext<'a>,
) -> Result<(), ExecutionError> {
    for step in steps {
        let step_id = step.get_step_id();
        context.add_step_started(step_id).await;
        match execute_step(step, context, step_id).await {
            Ok(_) => {
                context.add_step_finished(step_id).await;
            }
            Err(e) => {
                // Silence logging error that are in fact a exception
                match &e {
                    ExecutionError::Continue => {
                        context.add_step_finished(step.get_step_id()).await;
                    }
                    ExecutionError::Break => {
                        context.add_step_finished(step.get_step_id()).await;
                    }
                    ExecutionError::Return { .. } => {
                        context.add_step_finished(step.get_step_id()).await;
                    }
                    e => {
                        context
                            .add_error_thrown(step.get_step_id(), e.clone())
                            .await;
                        context.add_step_finished(step.get_step_id()).await;
                        context.bubbling = true;
                    }
                }
                return Err(e);
            }
        }
    }
    Ok(())
}

#[instrument(level = "debug", skip_all)]
pub async fn execute<'a>(
    steps: &Steps,
    context: &mut ExecutionContext<'a>,
) -> Result<(), ExecutionError> {
    debug!(
        "Initial storage:\n{}",
        serde_json::to_string_pretty(&context.storage).unwrap()
    );
    debug!(
        "Environment:\n{}",
        serde_json::to_string_pretty(&context.environment.variables).unwrap()
    );

    match execute_steps(steps, context).await {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod test_executor {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn test_records_step_events() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        let steps = vec![Step::Assign {
            id: "1".to_owned(),
            value: "\"hello\"".to_owned(),
            to: "result".to_owned(),
        }];

        execute(&steps, &mut context)
            .await
            .expect("Execution failed");

        // Remove timestamps to make test deterministic
        for e in context.log.events.iter_mut() {
            match e {
                ExecutionEvent::StepStarted { timestamp, .. } => *timestamp = 0,
                ExecutionEvent::StepFinished { timestamp, .. } => *timestamp = 0,
                _ => {}
            }
        }

        assert_eq!(
            context.log.events[0],
            ExecutionEvent::StepStarted {
                id: "1".to_owned(),
                timestamp: 0
            }
        );
        assert_eq!(
            context.log.events[2],
            ExecutionEvent::StepFinished {
                id: "1".to_owned(),
                timestamp: 0
            }
        );
        assert_eq!(context.log.events.len(), 3)
    }

    #[test]
    fn test_execution_log_serialization() {
        let mut log = ExecutionLog {
            status: ExecutionStatus::Started,
            initial_storage: describe(StorageValue::Object(ObjectBody::new())),
            events: vec![],
            function_id: "test".to_owned(),
            function_name: "Test function".to_owned(),
            started_at: 200100100,
            finished_at: Some(500100200),
            result: None,
        };
        log.events.push(ExecutionEvent::StepStarted {
            id: "1".to_owned(),
            timestamp: 100,
        });
        log.events.push(ExecutionEvent::StorageUpdated {
            id: "1".to_owned(),
            selector: vec![SelectorPart::Identifier("test".to_owned())],
            value: describe(StorageValue::Number(22.0)),
        });
        log.events.push(ExecutionEvent::StepFinished {
            id: "1".to_owned(),
            timestamp: 110,
        });

        let serialized = serde_json::to_value(&log).unwrap();
        let expected = json!(
            {
                "status": "started",
                "initialStorage": {
                    "type": "object",
                    "value": {}
                },
                "functionId": "test",
                "functionName": "Test function",
                "startedAt": 200100100,
                "finishedAt": 500100200,
                "events": [
                  { "type": "STEP_STARTED", "id": "1", "timestamp": 100 },
                  {
                    "type": "STORAGE_UPDATED",
                    "id": "1",
                    "selector": [{ "identifier": "test" }],
                    "value": {
                        "type": "numberValue",
                        "value": 22
                    }
                  },
                  { "type": "STEP_FINISHED", "id": "1", "timestamp": 110 }
                ]
              }

        );

        assert_eq!(serialized, expected);
    }

    #[test]
    fn test_define_function_step_serialization() {
        let step = Step::DefineFunction {
            name: "Some function".to_string(),
            id: "some".to_string(),
            parameters: vec![],
            exported: false,
            body: vec![Step::Break {
                id: "break".to_owned(),
            }],
        };

        let serialized = serde_json::to_value(step).unwrap();
        let expected = json!(
            {
                "type": "defineFunction",
                "id": "some",
                "name": "Some function",
                "parameters": [],
                "exported": false,
                "body": [{ "type": "break", "id": "break" }]
            }
        );
        assert_eq!(serialized, expected);
    }
}

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use std::vec;

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
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tracing::info;
use tracing::warn;
use tracing::{debug, instrument};

use crate::actions::actors;
use crate::actions::alarms;
use crate::actions::convert;
use crate::actions::crypto;
use crate::actions::fs;
use crate::actions::http_client::http_request;
use crate::actions::http_server::respond_with;
use crate::actions::http_server::respond_with_file_stream;
use crate::actions::http_server::setup_route;
use crate::actions::kv;
use crate::actions::os;
use crate::actions::postgres;
use crate::actions::pubsub;
use crate::actions::redis;
use crate::actions::time;
use crate::actions::wasm;
use crate::actions::websocket;
use crate::debug::DebugMessage;
use crate::debug::ExecutionLoggerHandle;
use crate::debug::LoggerMessage;
use crate::errors::ExecutionError;
use crate::evaluations::eval;
use crate::evaluations::eval_saver;
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
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum Step {
    Call {
        id: String,
        callee: Callee,
        parameters: Vec<Parameter>,
        store_as: Option<String>,
    },
    DefineFunction {
        id: String,
        parameters: Vec<ParameterDefinition>,
        #[serde(default = "default_exported")]
        exported: bool,
        body: Steps,
    },
    DefineNativeFunction {
        id: String,
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
        elze: Option<Steps>,
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
        ExecutionContext::new(
            self.actor_id.clone(),
            self.module.clone(),
            self.global.clone(),
            self.environment.clone(),
            &mut self.resources,
            ExecutionMode::Probe,
        )
    }
}

#[derive(Debug, Clone)]
pub enum Function {
    Normal { id: String, body: Steps },
    Native { id: String, native_id: String },
}

impl Function {
    pub fn get_id(&self) -> &str {
        match self {
            Function::Normal { id, .. } => id,
            Function::Native { id, .. } => id,
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

#[derive(Debug)]
pub struct Global {
    pub modules: RwLock<Vec<Arc<CompiledModule>>>,
    pub registry: ResourceRegistry,
    pub execution_logger: ExecutionLoggerHandle,
    pub pub_sub: Mutex<HashMap<String, tokio::sync::broadcast::Sender<StorageValue>>>,
}

#[derive(Debug)]
pub struct GlobalBuilder {
    modules: Vec<Arc<CompiledModule>>,
    registry: ResourceRegistry,
    execution_logger: ExecutionLoggerHandle,
    pub_sub: HashMap<String, tokio::sync::broadcast::Sender<StorageValue>>,
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
        }
    }
}

fn get_timestamp() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(n) => n.as_millis().try_into().unwrap_or_default(),
        Err(_) => 0,
    }
}

#[derive(Debug, Clone)]
pub struct DebuggerHandle {
    pub id: String,
    pub sender: mpsc::Sender<DebugMessage>,
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
}

// TODO: Figure out proper error handling for such cases
fn handle_logging_error(result: Result<(), TrySendError<DebugMessage>>) {
    match result {
        Ok(_) => {
            // No-op
        }
        Err(e) => match e {
            mpsc::error::TrySendError::Full(_) => {
                warn!("Logging channel is full")
            }
            mpsc::error::TrySendError::Closed(_) => {
                warn!("Logging channel is closed")
            }
        },
    }
}

impl<'a> ExecutionContext<'a> {
    pub fn new(
        actor_id: String,
        module: Arc<CompiledModule>,
        global: Arc<Global>,
        environment: Arc<Environment>,
        resources: &'a mut ActorResources,
        mode: ExecutionMode,
    ) -> Self {
        ExecutionContext {
            actor_id,
            log: ExecutionLog::default(),
            storage: HashMap::new(),
            resources,
            environment,
            global,
            module,
            bubbling: false,
            references: HashMap::new(),
            mode,
        }
    }

    fn report_verbose_event(&mut self, event: ExecutionEvent) {
        match &self.mode {
            ExecutionMode::Fast => {
                // No-op
            }
            ExecutionMode::Probe => {
                self.log.events.push(event);
            }
            ExecutionMode::Debug(debugger) => {
                handle_logging_error(debugger.sender.try_send(DebugMessage::AppendEvent {
                    event: event.clone(),
                }));
                self.log.events.push(event);
            }
        }
    }

    pub fn add_step_started(&mut self, id: &str) {
        self.report_verbose_event(ExecutionEvent::StepStarted {
            id: id.to_owned(),
            timestamp: get_timestamp(),
        });
    }

    pub fn add_step_finished(&mut self, id: &str) {
        self.report_verbose_event(ExecutionEvent::StepFinished {
            id: id.to_owned(),
            timestamp: get_timestamp(),
        });
    }

    pub fn add_enter_function(&mut self, function_id: String, initial_storage: ObjectBody) {
        self.report_verbose_event(ExecutionEvent::EnterFunction {
            function_id,
            initial_storage,
        });
    }

    pub fn add_leave_function(&mut self, function_id: String) {
        self.report_verbose_event(ExecutionEvent::LeaveFunction { function_id });
    }

    pub fn add_error_thrown(&mut self, id: &str, error: ExecutionError) {
        match &self.mode {
            ExecutionMode::Fast => {
                let event = ExecutionEvent::ErrorThrown {
                    id: id.to_owned(),
                    error,
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
                    error,
                    // No need to add snapshot in probe mode as the user can retrieve the context using events
                    snapshot: None,
                };
                self.log.events.push(event);
            }
            ExecutionMode::Debug(debugger) => {
                let event = ExecutionEvent::ErrorThrown {
                    id: id.to_owned(),
                    error,
                    // No need to add snapshot in probe mode as the user can retrieve the context using events
                    snapshot: None,
                };
                handle_logging_error(debugger.sender.try_send(DebugMessage::AppendEvent {
                    event: event.clone(),
                }));
                self.log.events.push(event);
            }
        }
    }

    pub fn start_report(&mut self) {
        self.log = ExecutionLog::started_with_initial_storage(self.storage.clone());

        if let ExecutionMode::Debug(debugger) = &self.mode {
            handle_logging_error(debugger.sender.try_send(DebugMessage::StartReport {
                started_at: self.log.started_at,
                initial_storage: self.log.initial_storage.clone(),
            }));
        }
    }

    pub fn end_report(&mut self, final_status: ExecutionStatus) {
        let now = get_timestamp();
        self.log.status = final_status;
        self.log.finished_at = Some(now);

        if let ExecutionMode::Debug(debugger) = &self.mode {
            handle_logging_error(debugger.sender.try_send(DebugMessage::EndReport {
                status: self.log.status.clone(),
                finished_at: now,
            }));
        }
    }

    pub fn add_to_storage(
        &mut self,
        id: &str,
        selector: Vec<SelectorPart>,
        value: StorageValue,
    ) -> Result<(), ExecutionError> {
        tel::save_to_storage(&selector, &mut self.storage, value.clone())
            .map_err(ExecutionError::from)?;

        self.report_verbose_event(ExecutionEvent::StorageUpdated {
            id: id.to_string(),
            selector,
            value,
        });

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
    pub initial_storage: ObjectBody,
    pub events: Vec<ExecutionEvent>,
    pub started_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ExecutionReportMetadata {
    Http { path: String, method: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionReport {
    pub module_id: String,
    pub module_version_id: String,
    pub environment_id: String,
    pub log: ExecutionLog,
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
        error: ExecutionError,
        snapshot: Option<ContextSnapshot>,
    },
    StorageUpdated {
        id: String,
        selector: Selector,
        value: StorageValue,
    },
    #[serde(rename_all = "camelCase")]
    EnterFunction {
        function_id: String,
        initial_storage: ObjectBody,
    },
    #[serde(rename_all = "camelCase")]
    LeaveFunction {
        function_id: String,
    },
}

impl Default for ExecutionLog {
    fn default() -> Self {
        ExecutionLog {
            events: Vec::new(),
            status: ExecutionStatus::Started,
            initial_storage: HashMap::new(),
            started_at: get_timestamp(),
            finished_at: None,
        }
    }
}

impl ExecutionLog {
    pub fn started_with_initial_storage(storage: ObjectBody) -> Self {
        ExecutionLog {
            status: ExecutionStatus::Started,
            initial_storage: storage,
            events: Vec::new(),
            started_at: get_timestamp(),
            finished_at: None,
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
    let store_as = store_as.or_else(|| {
        // TODO(pr0gramista)#saveAs: Remove this or_else block once the saveAs parameter is completely removed
        parameters.iter().find_map(|p| match p {
            Parameter::Tel { name, expression } if name == "saveAs" => Some(expression.as_str()),
            _ => None,
        })
    });

    match native_id {
        "wasm/run_wasi" => wasm::run_wasi(context, parameters, step_id, store_as).await?,
        "fs/open" => fs::open_file(context, parameters, step_id).await?,
        "fs/write_string" => fs::write_string(context, parameters, step_id).await?,
        "fs/read_to_string" => fs::read_to_string(context, parameters, step_id, store_as).await?,
        "alarms/set_alarm" => alarms::set_alarm(context, parameters, step_id).await?,
        "alarms/set_interval" => alarms::set_interval(context, parameters, step_id).await?,
        "alarms/cancel" => alarms::cancel_alarm(context, parameters, step_id).await?,
        "alarms/setup_cronjob" => alarms::setup_cronjob(context, parameters, step_id).await?,
        "time/sleep" => time::sleep(context, parameters, step_id).await?,
        "time/get_current_time" => {
            time::get_current_time(context, parameters, step_id, store_as).await?
        }
        "actors/spawn" => actors::spawn_actor(context, parameters, step_id, store_as).await?,
        "os/run_command" => os::run_command(context, parameters, step_id, store_as).await?,
        "os/read_environment_variable" => {
            os::read_environment_variable(context, parameters, step_id, store_as)?
        }
        "os/set_environment_variable" => {
            os::set_environment_variable(context, parameters, step_id)?
        }
        "actors/terminate" => actors::terminate(context, parameters, step_id).await?,
        "actors/send_command" => actors::send_command(context, parameters, step_id).await?,
        "actors/get_actor_id" => {
            actors::get_actor_id(context, parameters, step_id, store_as).await?
        }
        "actors/check_actor_exists" => {
            actors::check_actor_exists(context, parameters, step_id, store_as).await?
        }
        "http_client/request" => http_request(context, parameters, step_id, store_as).await?,
        "http_server/setup_route" => setup_route(context, parameters, step_id).await?,
        "http_server/respond_with" => respond_with(context, parameters, step_id).await?,
        "http_server/respond_with_file_stream" => {
            respond_with_file_stream(context, parameters, step_id).await?
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
        "convert/parse_json" => convert::parse_json(context, parameters, step_id, store_as)?,
        "convert/to_json" => convert::to_json(context, parameters, step_id, store_as)?,
        "convert/parse_urlencoded" => {
            convert::parse_urlencoded(context, parameters, step_id, store_as)?
        }
        "convert/to_urlencoded" => convert::to_urlencoded(context, parameters, step_id, store_as)?,
        "crypto/get_uuid_v4" => crypto::get_uuid_v4(context, parameters, step_id, store_as)?,
        "crypto/get_uuid_v7" => crypto::get_uuid_v7(context, parameters, step_id, store_as)?,
        "crypto/jwt_decode" => crypto::jwt_decode(context, parameters, step_id, store_as)?,
        "pubsub/publish" => pubsub::publish(context, parameters, step_id).await?,
        "pubsub/subscribe" => pubsub::subscribe(context, parameters, step_id).await?,
        "pubsub/unsubscribe" => pubsub::unsubscribe(context, parameters, step_id).await?,
        id => {
            return Err(ExecutionError::Unsupported {
                message: format!("Native function {} not found", id),
            });
        }
    }
    Ok(())
}

async fn execute_function<'a>(
    module: Arc<CompiledModule>,
    function_id: &str,
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    store_as: Option<&str>,
    step_id: &str,
) -> Result<(), ExecutionError> {
    let function = module
        .exported_functions
        .iter()
        .find(|f| f.get_id() == function_id)
        .or_else(|| {
            module
                .local_functions
                .iter()
                .find(|f| f.get_id() == function_id)
        });

    if let Some(function) = function {
        match function {
            Function::Normal { id: _, body } => {
                let mut initial_storage = HashMap::new();
                let mut initial_references = HashMap::new();

                let mut parameter_saver: Option<Selector> = None; // TODO(pr0gramista)#saveAs: Remove once the saveAs parameter is completely removed
                for parameter in parameters.iter() {
                    match parameter {
                        // TODO(pr0gramista)#saveAs: Remove once the saveAs parameter is completely removed
                        Parameter::Tel { name, expression } if name == "saveAs" => {
                            parameter_saver = Some(eval_saver(
                                expression,
                                &context.storage,
                                &context.environment,
                            )?);
                        }
                        Parameter::Tel { name, expression } => {
                            let value = eval(expression, &context.storage, &context.environment)?;
                            initial_storage.insert(name.clone(), value);
                        }
                        Parameter::FunctionRef { name, id } => {
                            initial_references.insert(name.clone(), id.clone());
                        }
                    }
                }

                context.add_enter_function(function_id.to_owned(), initial_storage.clone());

                let mut function_context = ExecutionContext {
                    actor_id: context.actor_id.clone(),
                    log: ExecutionLog::default(),
                    storage: initial_storage,
                    environment: context.environment.clone(),
                    resources: context.resources,
                    global: context.global.clone(),
                    module: module.clone(),
                    bubbling: false,
                    references: initial_references,
                    mode: context.mode.clone(),
                };

                let returned_value = match execute_steps(body, &mut function_context).await {
                    Ok(_) => StorageValue::Null(None),
                    Err(e) => match e {
                        ExecutionError::Return { value } => value,
                        e => {
                            context.log.events.append(&mut function_context.log.events);
                            context.add_leave_function(function_id.to_owned());
                            return Err(e);
                        }
                    },
                };

                context.log.events.append(&mut function_context.log.events);
                context.add_leave_function(function_id.to_owned());

                if let Some(expression) = store_as {
                    let selector = eval_saver(expression, &context.storage, &context.environment)?;
                    context.add_to_storage(step_id, selector, returned_value)?;
                    return Ok(());
                }

                if let Some(saver) = parameter_saver {
                    context.add_to_storage(step_id, saver, returned_value.clone())?;
                }
                Ok(())
            }
            Function::Native { id: _, native_id } => {
                match execute_native(native_id, context, parameters, store_as, step_id).await {
                    Ok(_) => Ok(()),
                    Err(e) => Err(e),
                }
            }
        }
    } else {
        Err(ExecutionError::FunctionNotFound {
            id: function_id.to_owned(),
        })
    }
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
            let selector = eval_saver(to, &context.storage, &context.environment)?;

            context.add_to_storage(step_id, selector, value)?;
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
            id: _,
            condition,
            then,
            elze,
        } => {
            let value = eval(condition, &context.storage, &context.environment)?;

            if value == StorageValue::Boolean(true) {
                execute_steps(then, context).await?;
            } else if let Some(elze) = elze {
                execute_steps(elze, context).await?;
            }
        }
        Step::ForEach {
            id: _,
            items,
            item,
            body,
        } => {
            let selector = eval_saver(item, &context.storage, &context.environment)?;
            let value = eval(items, &context.storage, &context.environment)?;

            match value {
                StorageValue::Array(arr) => {
                    for item in arr {
                        context.add_to_storage(step_id, selector.clone(), item)?;

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
                        context.add_to_storage(
                            step_id,
                            selector.clone(),
                            StorageValue::Object(vec![(key.clone(), value)].into_iter().collect()),
                        )?;

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
                        context.add_to_storage(
                            step_id,
                            selector.clone(),
                            StorageValue::String(c.to_string()),
                        )?;

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
            }
        }
        Step::While {
            id: _,
            condition,
            body,
        } => {
            while eval(condition, &context.storage, &context.environment)?
                == StorageValue::Boolean(true)
            {
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
                    let serialized = serde_json::to_value(&e).unwrap();

                    context.add_to_storage(
                        id,
                        vec![SelectorPart::Identifier("error".to_owned())],
                        serde_json::from_value(serialized).unwrap(),
                    )?;
                    execute_steps(catch, context).await?;
                }
            },
        },
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
        context.add_step_started(step_id);
        match execute_step(step, context, step_id).await {
            Ok(_) => {
                context.add_step_finished(step_id);
            }
            Err(e) => {
                // Silence logging error that are in fact a exception
                match &e {
                    ExecutionError::Continue => {
                        context.add_step_finished(step.get_step_id());
                    }
                    ExecutionError::Break => {
                        context.add_step_finished(step.get_step_id());
                    }
                    ExecutionError::Return { .. } => {
                        context.add_step_finished(step.get_step_id());
                    }
                    e => {
                        context.add_error_thrown(step.get_step_id(), e.clone());
                        context.add_step_finished(step.get_step_id());
                        context.bubbling = true;
                    }
                }
                return Err(e);
            }
        }
    }
    Ok(())
}

#[instrument(level = "info", skip_all)]
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

    context.start_report();

    match execute_steps(steps, context).await {
        Ok(_) => {
            context.end_report(ExecutionStatus::Finished);
            Ok(())
        }
        Err(e) => {
            context.end_report(ExecutionStatus::Failed);
            Err(e)
        }
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
        let mut log = ExecutionLog::default();
        log.events.push(ExecutionEvent::StepStarted {
            id: "1".to_owned(),
            timestamp: 100,
        });
        log.events.push(ExecutionEvent::StorageUpdated {
            id: "1".to_owned(),
            selector: vec![SelectorPart::Identifier("test".to_owned())],
            value: StorageValue::Number(22.0),
        });
        log.events.push(ExecutionEvent::StepFinished {
            id: "1".to_owned(),
            timestamp: 110,
        });

        let serialized = serde_json::to_value(&log).unwrap();
        let expected = json!(
            {
                "status": "started",
                "initialStorage": {},
                "startedAt": log.started_at,
                "events": [
                  { "type": "STEP_STARTED", "id": "1", "timestamp": 100 },
                  {
                    "type": "STORAGE_UPDATED",
                    "id": "1",
                    "selector": [{ "identifier": "test" }],
                    "value": 22
                  },
                  { "type": "STEP_FINISHED", "id": "1", "timestamp": 110 }
                ]
              }

        );

        assert_eq!(serialized, expected,);
    }

    #[test]
    fn test_define_function_step_serialization() {
        let step = Step::DefineFunction {
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
                "parameters": [],
                "exported": false,
                "body": [{ "type": "break", "id": "break" }]
            }
        );
        assert_eq!(serialized, expected);
    }
}

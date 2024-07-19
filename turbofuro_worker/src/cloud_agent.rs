use std::{collections::HashMap, sync::Arc};

use crate::{
    config::{fetch_configuration, ConfigurationCoordinator},
    environment_resolver::SharedEnvironmentResolver,
    errors::WorkerError,
    events::{WorkerEvent, WorkerEventReceiver},
    module_version_resolver::SharedModuleVersionResolver,
    options::CloudOptions,
    shared::{compile_module, get_compiled_module, ModuleVersion, WorkerStatus},
    VERSION,
};
use futures_util::{SinkExt, StreamExt};
use reqwest::Url;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc::{self, Sender};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{debug, error, info, info_span, instrument, warn, Instrument};
use turbofuro_runtime::{
    actor::{Actor, ActorCommand},
    debug::DebugMessage,
    executor::{
        evaluate_parameters, get_timestamp, Callee, DebugState, DebuggerHandle, Environment,
        ExecutionEvent, ExecutionLog, ExecutionStatus, Global, Import, Parameter,
    },
    handle_dangling_error,
    resources::{ActorLink, ActorResources},
    Description, ObjectBody, StorageValue,
};

#[derive(Debug)]
pub enum CloudAgentError {
    WebSocketError {
        error: tokio_tungstenite::tungstenite::Error,
    },
    InvalidOperatorUrl {
        url: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum OperatorCommand {
    #[serde(rename_all = "camelCase")]
    StartDebugReport {
        id: String,
        started_at: u64,
        initial_storage: Description,
        module_id: String,
        module_version_id: String,
        environment_id: String,
        function_id: String,
        function_name: String,
        status: ExecutionStatus,
        events: Vec<ExecutionEvent>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<Value>,
    },
    #[serde(rename_all = "camelCase")]
    AppendEventToDebugReport { id: String, event: ExecutionEvent },
    #[serde(rename_all = "camelCase")]
    EndDebugReport {
        id: String,
        status: ExecutionStatus,
        #[serde(rename = "finishedAt")]
        finished_at: u64,
    },
    #[serde(rename_all = "camelCase")]
    ReportEvent {
        id: String,
        started_at: u64,
        finished_at: u64,
        module_id: String,
        module_version_id: String,
        function_id: String,
        function_name: String,
        status: ExecutionStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<Value>,
    },
    #[serde(rename_all = "camelCase")]
    UpdateState {
        version: &'static str,
        os: &'static str,
        name: String,
        status: WorkerStatus,
        timestamp: u64,
    },
    #[serde(rename_all = "camelCase")]
    ReportError { id: String, error: WorkerError },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CloudAgentCommand {
    #[serde(rename_all = "camelCase")]
    RunFunction {
        id: String,
        module_version: ModuleVersion,
        callee: Callee,
        parameters: Vec<Parameter>,
    },
    #[serde(rename_all = "camelCase")]
    EnableDebugger { id: String, module_id: String },
    #[serde(rename_all = "camelCase")]
    DisableDebugger { id: String, module_id: String },
    #[serde(rename_all = "camelCase")]
    ReloadConfiguration { id: String },
    #[serde(rename_all = "camelCase")]
    ReloadEnvironment { id: String },
    #[serde(rename_all = "camelCase")]
    EnableActiveDebugger {
        id: String,
        module_version: ModuleVersion,
    },
    #[serde(rename_all = "camelCase")]
    ProlongActiveDebugger { id: String, module_id: String },
    #[serde(rename_all = "camelCase")]
    ReloadActiveDebugger {
        id: String,
        module_version: ModuleVersion,
    },
    #[serde(rename_all = "camelCase")]
    DisableActiveDebugger {
        id: String,
        module_version: ModuleVersion,
    },
}

pub struct CloudAgent {
    pub options: CloudOptions,
    pub environment_resolver: SharedEnvironmentResolver,
    pub module_version_resolver: SharedModuleVersionResolver,
    pub configuration_coordinator: ConfigurationCoordinator,
    pub global: Arc<Global>,
    pub debug_state: DebugState,
    pub worker_id: String,
    pub worker_event_receiver: WorkerEventReceiver,
    pub status: WorkerStatus,
}

fn wrap_debug_message(message: DebugMessage) -> OperatorCommand {
    match message {
        DebugMessage::StartReport {
            id,
            started_at,
            initial_storage,
            module_id,
            module_version_id,
            environment_id,
            function_id,
            function_name,
            status,
            events,
            metadata,
            finished_at: _, // This comes later in end report event
        } => OperatorCommand::StartDebugReport {
            id,
            started_at,
            initial_storage,
            module_id,
            module_version_id,
            environment_id,
            function_id,
            function_name,
            metadata,
            status,
            events,
        },
        DebugMessage::AppendEventToReport { id, event } => {
            OperatorCommand::AppendEventToDebugReport { id, event }
        }
        DebugMessage::EndReport {
            id,
            status,
            finished_at,
        } => OperatorCommand::EndDebugReport {
            id,
            status,
            finished_at,
        },
    }
}

fn spawn_debugger_handle_reader(
    mut receiver: tokio::sync::mpsc::Receiver<DebugMessage>,
    operator_sender: mpsc::Sender<OperatorCommand>,
) {
    tokio::spawn(async move {
        while let Some(message) = receiver.recv().await {
            let message = wrap_debug_message(message);
            match operator_sender.send(message).await {
                Ok(_) => {}
                Err(err) => {
                    // Quite simple cancellation - when WebSocket is closed, the write will fail and we will stop the loop
                    debug!("Cloud agent: Debug write failed: {}", err);
                    break;
                }
            }
        }
        debug!("Cloud agent: Debug writer finished");
    });
}

impl CloudAgent {
    fn get_operator_url(&self) -> Result<Url, CloudAgentError> {
        let query_params = serde_urlencoded::to_string([
            ("token", self.options.token.clone()),
            ("id", self.worker_id.clone()),
        ])
        .map_err(|_| CloudAgentError::InvalidOperatorUrl {
            url: self.options.operator_url.clone(),
        })?;
        let url_string = format!("{}/server?{}", self.options.operator_url, query_params,);
        Url::parse(&url_string).map_err(|_| CloudAgentError::InvalidOperatorUrl {
            url: url_string.clone(),
        })
    }

    async fn process_cloud_agent_command(
        &mut self,
        command: CloudAgentCommand,
        operator_sender: mpsc::Sender<OperatorCommand>,
    ) {
        match command {
            CloudAgentCommand::RunFunction {
                id,
                module_version,
                callee,
                parameters,
            } => {
                let (debugger_handle, receiver) = DebuggerHandle::new();
                spawn_debugger_handle_reader(receiver, operator_sender.clone());

                let function_id = match callee {
                    Callee::Local { function_id } => function_id,
                    Callee::Import {
                        import_name: _,
                        function_id: _,
                    } => {
                        handle_dangling_error!(
                            operator_sender
                                .send(OperatorCommand::ReportError {
                                    id: id.clone(),
                                    error: WorkerError::InvalidCloudAgentCommand {
                                        message:
                                            "Running imported functions is currently not supported"
                                                .to_owned(),
                                    },
                                })
                                .await
                        );
                        return;
                    }
                };

                match self
                    .perform_run(
                        id.clone(),
                        module_version,
                        &function_id,
                        parameters,
                        debugger_handle,
                    )
                    .await
                {
                    Ok(_) => {}
                    Err(err) => {
                        match err {
                            WorkerError::ExecutionFailed { error: _ } => {
                                // No-op as execution failure are still reported in reports
                            }
                            err => {
                                handle_dangling_error!(
                                    operator_sender
                                        .send(OperatorCommand::ReportError {
                                            id: id.clone(),
                                            error: err,
                                        })
                                        .await
                                )
                            }
                        }
                    }
                }
            }
            CloudAgentCommand::ReloadConfiguration { id: _ } => {
                debug!("Cloud agent: Received configuration reload command");
                let result = fetch_configuration(&self.options).await;
                match result {
                    Ok(configuration) => {
                        self.configuration_coordinator
                            .update_configuration(configuration)
                            .await;
                    }
                    Err(e) => {
                        warn!("Cloud agent: Failed to fetch configuration: {}", e);
                    }
                }
            }
            CloudAgentCommand::EnableDebugger { module_id, .. } => {
                let (debugger_handle, receiver) = DebuggerHandle::new();
                spawn_debugger_handle_reader(receiver, operator_sender.clone());

                if self.debug_state.has_entry(&module_id) {
                    warn!(
                        "Cloud agent: Debugger already enabled for module {}",
                        module_id.clone()
                    );
                    return;
                }

                self.debug_state
                    .add_entry(module_id.clone(), debugger_handle.clone());
                self.global
                    .debug_state
                    .store(Arc::new(self.debug_state.clone()));

                let actors_to_message = self
                    .global
                    .registry
                    .actors
                    .iter()
                    .filter(|item| {
                        let actor_link = item.value();
                        actor_link.module_id == module_id
                    })
                    .map(|item| item.value().clone())
                    .collect::<Vec<ActorLink>>();

                for actor_link in actors_to_message {
                    let _ = actor_link
                        .send(ActorCommand::EnableDebugger {
                            handle: debugger_handle.clone(),
                        })
                        .await;
                }
            }
            CloudAgentCommand::DisableDebugger { module_id, .. } => {
                self.debug_state.remove_entry(&module_id);
                self.global
                    .debug_state
                    .store(Arc::new(self.debug_state.clone()));

                let actors_to_message = self
                    .global
                    .registry
                    .actors
                    .iter()
                    .filter(|item| {
                        let actor_link = item.value();
                        actor_link.module_id == module_id
                    })
                    .map(|item| item.value().clone())
                    .collect::<Vec<ActorLink>>();

                for actor_link in actors_to_message {
                    let _ = actor_link.send(ActorCommand::DisableDebugger {}).await;
                }
            }
            CloudAgentCommand::ReloadEnvironment { id } => {
                debug!("Cloud agent: Received environment reload command");
                let environment: Environment = match self
                    .environment_resolver
                    .lock()
                    .await
                    .get_environment(&id)
                    .instrument(info_span!("get_environment"))
                    .await
                {
                    Ok(environment) => environment,
                    Err(e) => {
                        error!("Could not fetch environment: {:?}", e);
                        return;
                    }
                };

                let mut global_environment = self.global.environment.write().await;
                *global_environment = environment;
            }
            CloudAgentCommand::EnableActiveDebugger { .. } => {
                // todo!() but we don't want to it to crash right now
            }
            CloudAgentCommand::ProlongActiveDebugger { .. } => {
                // todo!() but we don't want to it to crash right now
            }
            CloudAgentCommand::ReloadActiveDebugger { .. } => {
                // todo!() but we don't want to it to crash right now
            }
            CloudAgentCommand::DisableActiveDebugger { .. } => {
                // todo!() but we don't want to it to crash right now
            }
        }
    }

    async fn process_message(
        &mut self,
        message: Message,
        operator_sender: mpsc::Sender<OperatorCommand>,
    ) {
        match message {
            Message::Text(payload) => {
                debug!("Cloud agent: Received command: {}", payload);
                let command: CloudAgentCommand = match serde_json::from_str(&payload) {
                    Ok(command) => command,
                    Err(err) => {
                        warn!("Cloud agent: error parsing command: {}", err);
                        return;
                    }
                };
                self.process_cloud_agent_command(command, operator_sender)
                    .await;
            }
            m => {
                warn!("Cloud agent: Received a unhandled message: {:?}", m);
            }
        }
    }

    async fn update_state(&mut self, write_sender: Sender<OperatorCommand>) {
        let _ = write_sender
            .send(OperatorCommand::UpdateState {
                version: VERSION,
                os: std::env::consts::OS,
                name: self.options.name.clone(),
                status: self.status.clone(),
                timestamp: get_timestamp(),
            })
            .await;
    }

    pub async fn start(mut self) -> Result<(), CloudAgentError> {
        info!("Cloud agent: Starting {}", self.options.name);

        let url = self.get_operator_url()?;
        let (ws_stream, _) = connect_async(url)
            .await
            .map_err(|e| CloudAgentError::WebSocketError { error: e })?;
        let (write, mut read) = ws_stream.split();
        let mut write = write.buffer(16);

        info!("Cloud agent: Connected to operator");

        // Start WebSocket command writer
        let (write_sender, mut write_receiver) = mpsc::channel::<OperatorCommand>(16);
        tokio::spawn(async move {
            while let Some(message) = write_receiver.recv().await {
                debug!("Cloud agent: Sending command to operator: {:?}", message);
                let message = Message::Text(serde_json::to_string(&message).unwrap());
                match write.send(message).await {
                    Ok(_) => {}
                    Err(err) => {
                        // Quite simple cancellation - when WebSocket is closed, the write will fail and we will stop the loop
                        debug!("Cloud agent: Write failed: {}", err);
                        break;
                    }
                }
            }
        });

        // Start WebSocket command reader
        while let Some(result) = read.next().await {
            match result {
                Ok(message) => self.process_message(message, write_sender.clone()).await,
                Err(e) => {
                    info!("Cloud agent: Error reading from server: {}", e);
                    break;
                }
            }
        }

        tokio::spawn(async move {
            while let Ok(event) = self.worker_event_receiver.recv().await {
                match event {
                    WorkerEvent::WorkerStarting => {
                        // Make sure to clear warnings
                        self.status = WorkerStatus::Starting { warnings: vec![] };
                        self.update_state(write_sender.clone()).await;
                    }
                    WorkerEvent::WorkerStarted => {
                        self.status = WorkerStatus::Running {
                            warnings: self.status.get_warnings(),
                        };
                        self.update_state(write_sender.clone()).await;
                    }
                    WorkerEvent::WorkerStopping(reason) => {
                        self.status = WorkerStatus::Stopping {
                            warnings: self.status.get_warnings(),
                            reason,
                        };
                        self.update_state(write_sender.clone()).await;
                    }
                    WorkerEvent::WorkerStopped(reason) => {
                        self.status = WorkerStatus::Stopped {
                            warnings: self.status.get_warnings(),
                            reason,
                        };
                        self.update_state(write_sender.clone()).await;
                    }
                    WorkerEvent::WarningRaised(warning) => {
                        self.status.add_warning(warning.clone());
                        self.update_state(write_sender.clone()).await;
                    }
                }
            }
            debug!("Cloud agent: Worker event receiver closed");
        });

        Ok(())
    }

    #[instrument(level = "info", skip_all)]
    async fn perform_run(
        &mut self,
        id: String,
        module_version: ModuleVersion,
        function_id: &str,
        parameters: Vec<Parameter>,
        debugger: DebuggerHandle,
    ) -> Result<ExecutionLog, WorkerError> {
        let global = self.global.clone();
        let environment = { self.global.environment.read().await.clone() };
        let module_version_resolver = self.module_version_resolver.clone();

        // Resolve module version for each import
        let mut imports = HashMap::new();
        for (import_name, import) in &module_version.imports {
            let imported = match import {
                Import::Cloud { id: _, version_id } => {
                    get_compiled_module(version_id, global.clone(), module_version_resolver.clone())
                        .await?
                }
            };
            imports.insert(import_name.to_owned(), imported);
        }

        let mut compiled_module = compile_module(module_version);
        compiled_module.imports = imports;
        let module = Arc::new(compiled_module);
        {
            global.modules.write().await.push(module.clone());
        }

        let mut actor = Actor::new(
            StorageValue::Null(None),
            Arc::new(environment.clone()),
            module.clone(),
            self.global.clone(),
            ActorResources::default(),
            HashMap::new(),
            Some(debugger),
        );

        let (storage, references) =
            evaluate_parameters(&parameters, &ObjectBody::new(), &environment)?;

        actor
            .execute_function(function_id, storage, references, Some(id))
            .await
            .map_err(WorkerError::from)
    }
}

#[cfg(test)]
mod test_cloud_agent {
    use std::collections::HashMap;

    use serde_json::json;
    use turbofuro_runtime::executor::Callee;

    use crate::shared::{WorkerStoppingReason, WorkerWarning};

    use super::*;

    #[test]
    fn test_run_function_command_serialization() {
        let command = CloudAgentCommand::RunFunction {
            id: "123".to_string(),
            module_version: ModuleVersion {
                module_id: "123".to_owned(),
                id: "123".to_owned(),
                instructions: vec![],
                handlers: HashMap::new(),
                imports: HashMap::new(),
            },
            callee: Callee::Local {
                function_id: "main".to_owned(),
            },
            // environment_id: Some("123".to_string()),
            parameters: vec![],
        };

        let serialized = serde_json::to_value(command).unwrap();
        let expected = json!(
            {
                "type": "runFunction",
                "id": "123",
                "moduleVersion": {
                    "moduleId": "123",
                    "id": "123",
                    "instructions": [],
                    "handlers": {},
                    "imports": {}
                },
                "callee": "local/main",
                "parameters": []
              }

        );
        assert_eq!(serialized, expected);
    }

    #[test]
    fn test_start_debug_report_serialization() {
        let command = OperatorCommand::StartDebugReport {
            id: "debug".to_string(),
            started_at: 200,
            initial_storage: Description::new_base_type("string"),
            module_id: "my_module".to_string(),
            module_version_id: "my_module_version".to_string(),
            environment_id: "my_environment".to_string(),
            function_id: "my_function".to_string(),
            function_name: "My function".to_string(),
            status: ExecutionStatus::Finished,
            events: vec![],
            metadata: json!({ "test": "test" }).into(),
        };

        let serialized = serde_json::to_value(command).unwrap();
        let expected = json!(
            {
                "type": "startDebugReport",
                "id": "debug",
                "startedAt": 200,
                "initialStorage": {
                    "type": "baseType",
                    "fieldType": "string"
                },
                "moduleId": "my_module",
                "moduleVersionId": "my_module_version",
                "environmentId": "my_environment",
                "functionId": "my_function",
                "functionName": "My function",
                "status": "finished",
                "events": [],
                "metadata": {
                    "test": "test"
                }
            }
        );
        assert_eq!(serialized, expected);
    }

    #[test]
    fn test_update_state_serialization() {
        let command = OperatorCommand::UpdateState {
            version: "1.0.0",
            os: "macos",
            name: "My worker".to_owned(),
            status: WorkerStatus::Running {
                warnings: vec![WorkerWarning::DebuggerActive {
                    modules: vec!["test".to_owned()],
                }],
            },
            timestamp: 200,
        };

        let serialized = serde_json::to_value(command).unwrap();
        let expected = json!(
            {
                "type": "updateState",
                "version": "1.0.0",
                "os": "macos",
                "name": "My worker",
                "status": {
                    "type": "running",
                    "warnings": [
                        {
                            "type": "debuggerActive",
                            "modules": ["test"]
                        }
                    ]
                },
                "timestamp": 200
            }
        );
        assert_eq!(serialized, expected);
    }

    #[test]
    fn test_update_state_stopping_serialization() {
        let command = OperatorCommand::UpdateState {
            version: "1.0.0",
            os: "macos",
            name: "My worker".to_owned(),
            status: WorkerStatus::Stopping {
                reason: WorkerStoppingReason::EnvironmentChanged,
                warnings: vec![],
            },
            timestamp: 200,
        };

        let serialized = serde_json::to_value(command).unwrap();
        let expected = json!(
            {
                "type": "updateState",
                "version": "1.0.0",
                "os": "macos",
                "name": "My worker",
                "status": {
                    "type": "stopping",
                    "reason": "ENVIRONMENT_CHANGED",
                    "warnings": []
                },
                "timestamp": 200
            }
        );
        assert_eq!(serialized, expected);
    }
}

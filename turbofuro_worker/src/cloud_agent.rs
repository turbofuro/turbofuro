use std::{collections::HashMap, sync::Arc};

use futures_util::{SinkExt, StreamExt};
use reqwest::Url;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

use tracing::{debug, info, instrument, warn};
use turbofuro_runtime::{
    actor::Actor,
    debug::DebugMessage,
    executor::{
        Callee, DebuggerHandle, ExecutionEvent, ExecutionLog, ExecutionStatus, Global, Import,
        Parameter, Step, Steps,
    },
    resources::ActorResources,
    Description, ObjectBody, StorageValue,
};

use crate::{
    config::{fetch_configuration, ConfigurationCoordinator},
    environment_resolver::SharedEnvironmentResolver,
    errors::WorkerError,
    module_version_resolver::SharedModuleVersionResolver,
    options::CloudOptions,
    shared::{compile_module, get_compiled_module, ModuleVersion},
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
pub enum MachineReloadingReason {
    ConfigurationChanged,
    EnvironmentChanged,
    RequestedByUser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MachineStoppingReason {
    SignalReceived,
    ConfigurationChanged,
    EnvironmentChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MachineErroredReason {
    PortTaken,
    ModuleCouldNotBeLoaded,
    EnvironmentCouldNotBeLoaded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]

pub enum MachineStatus {
    Running { configuration_override: bool },
    Starting,
    Stopping { reason: MachineStoppingReason },
    Errored { reason: MachineErroredReason },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum OperatorCommand<'a> {
    ReportDiagnostic {
        id: String,
        details: Value,
    },
    ReportTaskCompleted {
        id: String,
    },
    ReportTaskFailed {
        id: String,
        error: WorkerError,
    },
    StartReport {
        id: String,
        #[serde(rename = "startedAt")]
        started_at: u64,
        #[serde(rename = "initialStorage")]
        initial_storage: Description,
    },
    AppendReportEvent {
        id: String,
        event: ExecutionEvent,
    },
    EndReport {
        id: String,
        status: ExecutionStatus,
        #[serde(rename = "finishedAt")]
        finished_at: u64,
    },
    SendExecutionMetadata {
        // metadata: ExecutionMetadata,
    },
    UpdateState {
        version: &'static str,
        os: &'static str,
        name: &'a str,
        status: MachineStatus,
        timestamp: u64,
    },
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
    EnableDebugger {
        id: String,
        module_id: String,
    },
    DisableDebugger {
        id: String,
        module_id: String,
    },
    ReloadConfiguration {
        id: String,
    },
    ReloadEnvironment {
        id: String,
    },
    EnableActiveDebugger {
        id: String,
        module_version: ModuleVersion,
    },
    ProlongActiveDebugger {
        id: String,
        module_id: String,
    },
    ReloadActiveDebugger {
        id: String,
        module_version: ModuleVersion,
    },
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
}

fn wrap_debug_message(message: DebugMessage, id: String) -> OperatorCommand<'static> {
    match message {
        DebugMessage::StartReport {
            started_at,
            initial_storage,
        } => OperatorCommand::StartReport {
            id,
            started_at,
            initial_storage,
        },
        DebugMessage::AppendEvent { event } => OperatorCommand::AppendReportEvent { id, event },
        DebugMessage::EndReport {
            status,
            finished_at,
        } => OperatorCommand::EndReport {
            id,
            status,
            finished_at,
        },
    }
}

impl CloudAgent {
    fn get_operator_url(&self) -> Result<Url, CloudAgentError> {
        let query_params = serde_urlencoded::to_string([
            ("token", self.options.token.clone()),
            ("name", self.options.name.clone()),
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
        debug_writer: mpsc::Sender<Message>,
    ) {
        match command {
            CloudAgentCommand::RunFunction {
                id,
                module_version,
                callee,
                parameters,
            } => {
                let (debugger_handle, mut receiver) = DebuggerHandle::new(id.clone());
                let log_writer = debug_writer.clone();
                let id_receiver = id.clone();
                tokio::spawn(async move {
                    while let Some(message) = receiver.recv().await {
                        let message = wrap_debug_message(message, id_receiver.clone());

                        match log_writer
                            .send(Message::Text(serde_json::to_string(&message).unwrap()))
                            .await
                        {
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

                match self
                    .perform_run(
                        id.clone(),
                        module_version,
                        callee,
                        parameters,
                        // environment_id,
                        debugger_handle,
                    )
                    .await
                {
                    Ok(_) => {
                        match debug_writer
                            .send(Message::Text(
                                serde_json::to_string(&OperatorCommand::ReportTaskCompleted { id })
                                    .expect("Failed to serialize report task completed"),
                            ))
                            .await
                        {
                            Ok(_) => {}
                            Err(e) => {
                                warn!("Cloud agent: Failed to send task completed: {}", e);
                            }
                        }
                    }
                    Err(error) => {
                        match debug_writer
                            .send(Message::Text(
                                serde_json::to_string(&OperatorCommand::ReportTaskFailed {
                                    id,
                                    error,
                                })
                                .expect("Failed to serialize task failed"),
                            ))
                            .await
                        {
                            Ok(_) => {}
                            Err(e) => {
                                warn!("Cloud agent: Failed to send task failed: {}", e);
                            }
                        }
                    }
                }
            }
            CloudAgentCommand::ReloadConfiguration { id: _ } => {
                debug!("Cloud agent: Received configuration update command");
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
            CloudAgentCommand::EnableDebugger { id, module_id } => todo!(),
            CloudAgentCommand::DisableDebugger { id, module_id } => todo!(),
            CloudAgentCommand::ReloadEnvironment { id } => todo!(),
            CloudAgentCommand::EnableActiveDebugger { id, module_version } => todo!(),
            CloudAgentCommand::ProlongActiveDebugger { id, module_id } => todo!(),
            CloudAgentCommand::ReloadActiveDebugger { id, module_version } => todo!(),
            CloudAgentCommand::DisableActiveDebugger { id, module_version } => todo!(),
        }
    }

    async fn process_message(&mut self, message: Message, debug_writer: mpsc::Sender<Message>) {
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
                self.process_cloud_agent_command(command, debug_writer)
                    .await;
            }
            m => {
                warn!("Cloud agent: Received a unhandled message: {:?}", m);
            }
        }
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

        // Start writer
        let (write_sender, mut write_receiver) = mpsc::channel(16);
        tokio::spawn(async move {
            while let Some(message) = write_receiver.recv().await {
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

        // Start reader
        while let Some(result) = read.next().await {
            match result {
                Ok(message) => self.process_message(message, write_sender.clone()).await,
                Err(e) => {
                    info!("Cloud agent: Error reading from server: {}", e);
                    break;
                }
            }
        }

        Ok(())
    }

    #[instrument(level = "info", skip_all)]
    async fn perform_run(
        &mut self,
        _id: String,
        module_version: ModuleVersion,
        callee: Callee,
        parameters: Vec<Parameter>,
        // environment_id: Option<String>,
        debugger_handle: DebuggerHandle,
    ) -> Result<ExecutionLog, WorkerError> {
        let global = self.global.clone();
        // let environment: Environment = match environment_id {
        //     Some(environment_id) => {
        //         self.environment_resolver
        //             .lock()
        //             .await
        //             .get_environment(&environment_id)
        //             .await?
        //     }
        //     None => self.global.environment.read().await.clone(),
        // };
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

        let mut actor = Actor::new_module_initiator(
            StorageValue::Null(None),
            Arc::new(environment),
            module.clone(),
            self.global.clone(),
            ActorResources::default(),
        );

        let custom_steps: Steps = vec![Step::Call {
            id: "debug".to_owned(),
            callee,
            parameters,
            store_as: None,
        }];

        let resources = ActorResources::default();
        let execution_log = actor
            .execute_custom(&custom_steps, resources, ObjectBody::new(), debugger_handle)
            .await;

        Ok(execution_log)
    }
}

#[cfg(test)]
mod test_cloud_agent {
    use std::collections::HashMap;

    use serde_json::json;
    use turbofuro_runtime::executor::Callee;

    use super::*;

    #[test]
    fn test_command_serialization() {
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
}

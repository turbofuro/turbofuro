use std::{collections::HashMap, env, sync::Arc};

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
    cloud_logger::RUNS_ACCUMULATOR,
    config::{fetch_configuration, ConfigurationCoordinator},
    environment_resolver::SharedEnvironmentResolver,
    module_version_resolver::SharedModuleVersionResolver,
    worker::{compile_module, get_compiled_module, AppError, ModuleVersion},
    CloudOptions,
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
pub enum OperatorCommand<'a> {
    ReportDiagnostic {
        id: String,
        details: Value,
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
    UpdateStats {
        os: &'static str,
        #[serde(rename = "runsCount")]
        runs_count: u64,
        name: &'a str,
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
        environment_id: String,
        // machine_id: String
    },
    UpdateConfiguration {
        id: String,
    },
}

pub struct CloudAgent {
    pub cloud_options: CloudOptions,
    pub environment_resolver: SharedEnvironmentResolver,
    pub module_version_resolver: SharedModuleVersionResolver,
    pub global: Arc<Global>,
    pub configuration_coordinator: ConfigurationCoordinator,
    pub name: String,
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
    pub async fn start(mut self) -> Result<(), CloudAgentError> {
        info!("Cloud agent: Starting {}", self.name);

        let url_string = format!(
            "{}/server?token={}",
            self.cloud_options.operator_url, self.cloud_options.token
        );
        let url = Url::parse(&url_string).map_err(|_| CloudAgentError::InvalidOperatorUrl {
            url: url_string.clone(),
        })?;

        let (ws_stream, _) = connect_async(url)
            .await
            .map_err(|e| CloudAgentError::WebSocketError { error: e })?;
        let (mut write, mut read) = ws_stream.split();
        let mut write = write.buffer(16);

        info!("Cloud agent: Connected to operator");

        // Start writer
        let (write_send, mut write_receiver) = mpsc::channel(16);
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

        // Setup stats reporting
        let report_write = write_send.clone();
        let name = self.name.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));

            loop {
                interval.tick().await;

                match {
                    debug!(
                        "Cloud agent: Sending stats {}",
                        serde_json::to_string(&OperatorCommand::UpdateStats {
                            os: env::consts::OS,
                            runs_count: RUNS_ACCUMULATOR.load(std::sync::atomic::Ordering::SeqCst),
                            name: &name,
                        })
                        .unwrap()
                    );
                    report_write
                        .send(Message::Text(
                            serde_json::to_string(&OperatorCommand::UpdateStats {
                                os: env::consts::OS,
                                name: &name,
                                runs_count: RUNS_ACCUMULATOR
                                    .load(std::sync::atomic::Ordering::SeqCst),
                            })
                            .unwrap(),
                        ))
                        .await
                } {
                    Ok(_) => {}
                    Err(err) => {
                        // Quite simple cancellation - when WebSocket is closed, the write will fail and we will stop the loop
                        debug!("Cloud agent: Stats write failed: {}", err);
                        break;
                    }
                }
            }
        });

        let debug_writer = write_send.clone();
        while let Some(result) = read.next().await {
            match result {
                Ok(command) => match command {
                    Message::Text(payload) => {
                        debug!("Cloud agent: Received command: {}", payload);
                        let command: CloudAgentCommand = match serde_json::from_str(&payload) {
                            Ok(command) => command,
                            Err(err) => {
                                warn!("Cloud agent: error parsing command: {}", err);
                                continue;
                            }
                        };

                        match command {
                            CloudAgentCommand::RunFunction {
                                id,
                                module_version,
                                callee,
                                parameters,
                                environment_id,
                            } => {
                                let (sender, mut receiver) = mpsc::channel::<DebugMessage>(16);

                                let debugger_handle = DebuggerHandle {
                                    sender,
                                    id: id.clone(),
                                };

                                let log_writer = debug_writer.clone();
                                let id_receiver = id.clone();
                                tokio::spawn(async move {
                                    while let Some(message) = receiver.recv().await {
                                        let message =
                                            wrap_debug_message(message, id_receiver.clone());

                                        match log_writer
                                            .send(Message::Text(
                                                serde_json::to_string(&message).unwrap(),
                                            ))
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

                                // TODO: Report app errors
                                let _result = self
                                    .perform_run(
                                        id.clone(),
                                        module_version,
                                        callee,
                                        parameters,
                                        environment_id,
                                        debugger_handle,
                                    )
                                    .await;
                            }
                            CloudAgentCommand::UpdateConfiguration { id: _ } => {
                                debug!("Cloud agent: Received configuration update command");
                                let result = fetch_configuration(&self.cloud_options).await;
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
                        }
                    }
                    m => {
                        warn!("Cloud agent: Received a unhandled message: {:?}", m);
                    }
                },
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
        environment_id: String,
        debugger_handle: DebuggerHandle,
    ) -> Result<ExecutionLog, AppError> {
        let environment = self
            .environment_resolver
            .lock()
            .await
            .get_environment(&environment_id)
            .await?;

        let global = self.global.clone();
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
            environment_id: "123".to_string(),
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
                "parameters": [],
                "environmentId": "123"
              }

        );
        assert_eq!(serialized, expected);
    }
}

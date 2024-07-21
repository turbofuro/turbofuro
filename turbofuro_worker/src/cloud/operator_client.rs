use std::time::Duration;

use crate::{
    errors::WorkerError,
    shared::{ModuleVersion, WorkerStatus},
    utils::exponential_delay::ExponentialDelay,
};
use futures_util::{SinkExt, StreamExt};
use log::info;
use reqwest::Url;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc::{self};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{debug, warn};
use turbofuro_runtime::{
    debug::DebugMessage,
    executor::{Callee, ExecutionEvent, ExecutionStatus, Parameter},
    Description,
};

use super::agent::CloudAgentHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SendingCommand {
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
pub enum ReceivingCommand {
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

static PING_PAYLOAD: &[u8; 4] = b"ping";

#[derive(Debug)]
pub enum OperatorClientError {
    WebSocketError {
        error: tokio_tungstenite::tungstenite::Error,
    },
}

struct OperatorClient {
    operator_url: Url,
    cloud_agent_handler: CloudAgentHandle,
    main_receiver: mpsc::Receiver<OperatorClientMessage>,

    child_handle: OperatorClientHandle,
    child_receiver: mpsc::Receiver<OperatorClientMessage>,

    ws_socket_writer: Option<mpsc::Sender<SendingCommand>>,

    reconnect_delay: ExponentialDelay,
}

#[derive(Debug)]
enum OperatorClientMessage {
    Connect,
    ReconnectWithDelay,
    SendCommand { command: SendingCommand },
    ReceiveCommand { command: ReceivingCommand },
}

impl From<DebugMessage> for SendingCommand {
    fn from(message: DebugMessage) -> Self {
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
            } => SendingCommand::StartDebugReport {
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
                SendingCommand::AppendEventToDebugReport { id, event }
            }
            DebugMessage::EndReport {
                id,
                status,
                finished_at,
            } => SendingCommand::EndDebugReport {
                id,
                status,
                finished_at,
            },
        }
    }
}

impl OperatorClient {
    fn new(
        main_receiver: mpsc::Receiver<OperatorClientMessage>,
        operator_url: Url,
        cloud_agent_handler: CloudAgentHandle,
    ) -> Self {
        let (child_sender, child_receiver) = mpsc::channel(16);

        OperatorClient {
            main_receiver,
            child_handle: OperatorClientHandle {
                sender: child_sender,
            },
            child_receiver,
            ws_socket_writer: None,
            operator_url,
            cloud_agent_handler,
            reconnect_delay: ExponentialDelay::default(),
        }
    }

    async fn handle_message(&mut self, message: OperatorClientMessage) {
        match message {
            OperatorClientMessage::Connect => {
                match connect_async(self.operator_url.clone())
                    .await
                    .map_err(|e| OperatorClientError::WebSocketError { error: e })
                {
                    Ok((ws_stream, _response)) => {
                        info!("Connected to operator");
                        let (write, mut read) = ws_stream.split();
                        let mut write = write.buffer(16);

                        let (ws_command_sender, mut ws_command_receiver) =
                            mpsc::channel::<SendingCommand>(16);
                        self.ws_socket_writer = Some(ws_command_sender);

                        self.reconnect_delay.reset();

                        // WebSocket reader
                        let handle_for_websocket = self.child_handle.clone();
                        tokio::spawn(async move {
                            while let Some(message) = read.next().await {
                                match message {
                                    Ok(message) => match message {
                                        Message::Text(text) => {
                                            let command: ReceivingCommand =
                                                match serde_json::from_str(&text) {
                                                    Ok(command) => command,
                                                    Err(err) => {
                                                        warn!("Could not parse command: {}", err);
                                                        continue;
                                                    }
                                                };

                                            handle_for_websocket.handle_command(command).await;
                                        }
                                        Message::Pong(_) => {
                                            // Do nothing
                                        }
                                        Message::Ping(_) => {
                                            // Do nothing
                                        }
                                        Message::Close(_) => {
                                            // Do nothing
                                        }
                                        message => {
                                            warn!("Unsupported message type {:?}", message);
                                        }
                                    },
                                    Err(e) => {
                                        warn!("Error reading from operator WebSocket ({:?})", e);
                                        handle_for_websocket.reconnect_with_delay().await;
                                    }
                                }
                            }
                        });

                        // WebSocket writer
                        tokio::spawn(async move {
                            let mut flush_interval =
                                tokio::time::interval(Duration::from_millis(200));
                            let mut keep_alive_interval =
                                tokio::time::interval(Duration::from_secs(60));

                            loop {
                                tokio::select! {
                                    command = ws_command_receiver.recv() => {
                                        match command {
                                            Some(command) => {
                                                let text = serde_json::to_string(&command).unwrap();
                                                let _ = write.feed(Message::Text(text)).await;
                                            }
                                            None => {
                                                break;
                                            }
                                        }
                                    }
                                    _ = flush_interval.tick() => {
                                        let _ = write.flush().await;
                                    }
                                    _ = keep_alive_interval.tick() => {
                                        let _ = write.send(Message::Ping(PING_PAYLOAD.into())).await;
                                    }
                                };
                            }
                        });
                    }
                    Err(e) => {
                        warn!("Failed to connect to operator ({:?})", e);
                        self.child_handle.reconnect_with_delay().await;
                    }
                }
            }
            OperatorClientMessage::SendCommand { command } => {
                if let Some(writer) = &mut self.ws_socket_writer {
                    match writer.send(command).await {
                        Ok(_) => {}
                        Err(e) => {
                            warn!("Failed to send command: {}", e);
                        }
                    }
                } else {
                    debug!("Operator WebSocket is not connected, dropping command");
                }
            }
            OperatorClientMessage::ReceiveCommand { command } => match command {
                ReceivingCommand::RunFunction {
                    id,
                    module_version,
                    callee,
                    parameters,
                } => {
                    let _ = self
                        .cloud_agent_handler
                        .perform_run(id, module_version, callee, parameters)
                        .await;
                }
                ReceivingCommand::EnableDebugger { module_id, .. } => {
                    let _ = self.cloud_agent_handler.enable_debugger(module_id).await;
                }
                ReceivingCommand::DisableDebugger { module_id, .. } => {
                    let _ = self.cloud_agent_handler.disable_debugger(module_id).await;
                }
                ReceivingCommand::ReloadConfiguration { .. } => {
                    let _ = self.cloud_agent_handler.reload_configuration().await;
                }
                ReceivingCommand::ReloadEnvironment { .. } => {
                    let _ = self.cloud_agent_handler.reload_environment().await;
                }
                ReceivingCommand::EnableActiveDebugger { .. } => {
                    // todo!(), but let's not panic
                    warn!("Enable active debugger not implemented");
                }
                ReceivingCommand::ProlongActiveDebugger { .. } => {
                    // todo!(), but let's not panic
                    warn!("Prolong active debugger not implemented");
                }
                ReceivingCommand::ReloadActiveDebugger { .. } => {
                    // todo!(), but let's not panic
                    warn!("Reload active debugger not implemented");
                }
                ReceivingCommand::DisableActiveDebugger { .. } => {
                    // todo!(), but let's not panic
                    warn!("Disable active debugger not implemented");
                }
            },
            OperatorClientMessage::ReconnectWithDelay => {
                let delay = self.reconnect_delay.next();

                debug!("Operator reconnecting in {:?}", delay);

                // Retry in 60 seconds
                let handle_for_retry = self.child_handle.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    handle_for_retry.connect().await;
                });
            }
        }
    }
}

async fn run_operator_client(mut actor: OperatorClient) {
    loop {
        tokio::select! {
            message = actor.main_receiver.recv() => {
                match message {
                    Some(message) => actor.handle_message(message).await,
                    None => {
                        break;
                    }
                }
            },
            message = actor.child_receiver.recv() => {
                match message {
                    Some(message) => actor.handle_message(message).await,
                    None => {
                        break;
                    }
                }
            },
        }
    }
}

#[derive(Clone)]
pub struct OperatorClientHandle {
    sender: mpsc::Sender<OperatorClientMessage>,
}

impl OperatorClientHandle {
    pub fn new(operator_url: Url, cloud_agent_handler: CloudAgentHandle) -> Self {
        let (sender, receiver) = mpsc::channel(16);
        let actor = OperatorClient::new(receiver, operator_url, cloud_agent_handler);
        tokio::spawn(run_operator_client(actor));
        Self { sender }
    }

    pub async fn connect(&self) {
        let msg = OperatorClientMessage::Connect;
        let _ = self.sender.send(msg).await;
    }

    pub async fn reconnect_with_delay(&self) {
        let msg = OperatorClientMessage::ReconnectWithDelay;
        let _ = self.sender.send(msg).await;
    }

    pub async fn send_command(&self, command: SendingCommand) {
        let msg = OperatorClientMessage::SendCommand { command };
        let _ = self.sender.send(msg).await;
    }

    pub async fn handle_command(&self, command: ReceivingCommand) {
        let msg = OperatorClientMessage::ReceiveCommand { command };
        let _ = self.sender.send(msg).await;
    }
}

#[cfg(test)]
mod test_operator_client {
    use std::collections::HashMap;

    use serde_json::json;
    use turbofuro_runtime::executor::Callee;

    use crate::shared::{WorkerStoppingReason, WorkerWarning};

    use super::*;

    #[test]
    fn test_run_function_command_serialization() {
        let command = ReceivingCommand::RunFunction {
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
        let command = SendingCommand::StartDebugReport {
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
        let command = SendingCommand::UpdateState {
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
        let command = SendingCommand::UpdateState {
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

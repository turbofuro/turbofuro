use std::collections::HashMap;
use std::sync::Arc;

use http::Uri;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, ClientRequestBuilder, Message as TungsteniteMessage},
};
use tokio_util::bytes::Bytes;
use tracing::{debug, instrument};

use crate::{
    actor::ActorCommand,
    errors::ExecutionError,
    evaluations::{
        eval_optional_param_with_default, eval_param, eval_string_param,
        get_optional_handler_from_parameters,
    },
    executor::{ExecutionContext, Global, Parameter},
    resources::{generate_resource_id, Cancellation, CancellationSubject, Resource, ResourceId},
};
use tel::{describe, Description, StorageValue};

pub const WEBSOCKET_CLIENT_RESOURCE_TYPE: &str = "websocket_client";

#[derive(Debug)]
pub struct WebSocketClientCommand {
    pub message: TungsteniteMessage,
    pub responder: oneshot::Sender<Result<(), ExecutionError>>,
}

impl WebSocketClientCommand {
    pub fn new(
        message: TungsteniteMessage,
    ) -> (Self, oneshot::Receiver<Result<(), ExecutionError>>) {
        let (responder, receiver) = oneshot::channel();
        let command = Self { message, responder };
        (command, receiver)
    }
}

#[derive(Debug)]
pub struct OpenWebSocketClient(pub ResourceId, pub mpsc::Sender<WebSocketClientCommand>);

impl Resource for OpenWebSocketClient {
    fn static_type() -> &'static str {
        WEBSOCKET_CLIENT_RESOURCE_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.0
    }
}

fn cancellation_name(url: &str) -> String {
    format!(
        "websocket_client_{}",
        url.replace("://", "_").replace("/", "_")
    )
}

async fn handle_websocket_messages(
    global: Arc<Global>,
    actor_id: String,
    url: String,
    ws_receiver: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    mut command_receiver: mpsc::Receiver<WebSocketClientCommand>,
    on_message_handler: Option<String>,
    on_close_handler: Option<String>,
    on_error_handler: Option<String>,
    cancel_receiver: oneshot::Receiver<()>,
) {
    use futures_util::{SinkExt, StreamExt};

    let (mut ws_sender, mut ws_receiver_stream) = ws_receiver.split();

    // Task to handle outgoing messages
    let mut command_task = tokio::spawn(async move {
        while let Some(command) = command_receiver.recv().await {
            match ws_sender.send(command.message).await {
                Ok(_) => {
                    let _ = command.responder.send(Ok(()));
                }
                Err(e) => {
                    let _ = command.responder.send(Err(ExecutionError::StateInvalid {
                        message: "Failed to send message to WebSocket".to_owned(),
                        subject: OpenWebSocketClient::static_type().into(),
                        inner: e.to_string(),
                    }));
                }
            }
        }
    });

    // Task to handle incoming messages
    let mut message_task = tokio::spawn(async move {
        while let Some(msg) = ws_receiver_stream.next().await {
            match msg {
                Ok(msg) => {
                    match msg {
                        TungsteniteMessage::Close(_) => {
                            let mut initial_storage = HashMap::new();
                            initial_storage.insert(
                                "reason".to_string(),
                                StorageValue::String("closed".to_owned()),
                            );

                            if let Some(handler) = &on_close_handler {
                                if let Some(messenger) = global
                                    .registry
                                    .actors
                                    .get(&actor_id)
                                    .map(|r| r.value().clone())
                                {
                                    let _ = messenger
                                        .send(ActorCommand::RunFunctionRef {
                                            function_ref: handler.clone(),
                                            storage: initial_storage,
                                            references: HashMap::new(),
                                            sender: None,
                                            execution_id: None,
                                        })
                                        .await;
                                }
                            } else if let Some(messenger) = global
                                .registry
                                .actors
                                .get(&actor_id)
                                .map(|r| r.value().clone())
                            {
                                let _ = messenger
                                    .send(ActorCommand::Run {
                                        handler: "onClose".to_owned(),
                                        storage: initial_storage,
                                        references: HashMap::new(),
                                        sender: None,
                                        execution_id: None,
                                    })
                                    .await;
                            }
                            break;
                        }
                        TungsteniteMessage::Text(text) => {
                            let mut initial_storage = HashMap::new();
                            initial_storage.insert(
                                "message".to_string(),
                                StorageValue::String(text.to_string()),
                            );

                            if let Some(handler) = &on_message_handler {
                                if let Some(messenger) = global
                                    .registry
                                    .actors
                                    .get(&actor_id)
                                    .map(|r| r.value().clone())
                                {
                                    let _ = messenger
                                        .send(ActorCommand::RunFunctionRef {
                                            function_ref: handler.clone(),
                                            storage: initial_storage,
                                            references: HashMap::new(),
                                            sender: None,
                                            execution_id: None,
                                        })
                                        .await;
                                }
                            } else if let Some(messenger) = global
                                .registry
                                .actors
                                .get(&actor_id)
                                .map(|r| r.value().clone())
                            {
                                let _ = messenger
                                    .send(ActorCommand::Run {
                                        handler: "onMessage".to_owned(),
                                        storage: initial_storage,
                                        references: HashMap::new(),
                                        sender: None,
                                        execution_id: None,
                                    })
                                    .await;
                            }
                        }
                        TungsteniteMessage::Binary(bytes) => {
                            let mut initial_storage = HashMap::new();
                            initial_storage.insert(
                                "message".to_string(),
                                StorageValue::Array(
                                    bytes
                                        .iter()
                                        .map(|b| StorageValue::Number(*b as f64))
                                        .collect(),
                                ),
                            );

                            if let Some(handler) = &on_message_handler {
                                if let Some(messenger) = global
                                    .registry
                                    .actors
                                    .get(&actor_id)
                                    .map(|r| r.value().clone())
                                {
                                    let _ = messenger
                                        .send(ActorCommand::RunFunctionRef {
                                            function_ref: handler.clone(),
                                            storage: initial_storage,
                                            references: HashMap::new(),
                                            sender: None,
                                            execution_id: None,
                                        })
                                        .await;
                                }
                            } else if let Some(messenger) = global
                                .registry
                                .actors
                                .get(&actor_id)
                                .map(|r| r.value().clone())
                            {
                                let _ = messenger
                                    .send(ActorCommand::Run {
                                        handler: "onMessage".to_owned(),
                                        storage: initial_storage,
                                        references: HashMap::new(),
                                        sender: None,
                                        execution_id: None,
                                    })
                                    .await;
                            }
                        }
                        TungsteniteMessage::Ping(_) => {
                            // Auto-respond to ping
                        }
                        TungsteniteMessage::Pong(_) => {
                            // Ignore pong
                        }
                        TungsteniteMessage::Frame(_) => {
                            // Ignore raw frames
                        }
                    }
                }
                Err(error) => {
                    let mut initial_storage = HashMap::new();
                    initial_storage
                        .insert("error".to_string(), StorageValue::String(error.to_string()));

                    if let Some(handler) = &on_error_handler {
                        if let Some(messenger) = global
                            .registry
                            .actors
                            .get(&actor_id)
                            .map(|r| r.value().clone())
                        {
                            let _ = messenger
                                .send(ActorCommand::RunFunctionRef {
                                    function_ref: handler.clone(),
                                    storage: initial_storage,
                                    references: HashMap::new(),
                                    sender: None,
                                    execution_id: None,
                                })
                                .await;
                        }
                    } else if let Some(messenger) = global
                        .registry
                        .actors
                        .get(&actor_id)
                        .map(|r| r.value().clone())
                    {
                        let _ = messenger
                            .send(ActorCommand::Run {
                                handler: "onError".to_owned(),
                                storage: initial_storage,
                                references: HashMap::new(),
                                sender: None,
                                execution_id: None,
                            })
                            .await;
                    }
                    break;
                }
            }
        }
    });

    // Wait for either cancellation or completion
    tokio::select! {
        _ = cancel_receiver => {
            debug!("WebSocket client connection cancelled for {}", url);
            command_task.abort();
            message_task.abort();
        }
        _ = &mut command_task => {
            debug!("WebSocket client command task ended for {}", url);
            message_task.abort();
        }
        _ = &mut message_task => {
            debug!("WebSocket client message task ended for {}", url);
            command_task.abort();
        }
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn connect<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let url = eval_string_param("url", parameters, context)?;
    let on_message_handler = get_optional_handler_from_parameters("onMessage", parameters);
    let on_close_handler = get_optional_handler_from_parameters("onClose", parameters);
    let on_error_handler = get_optional_handler_from_parameters("onError", parameters);
    let headers_param = eval_optional_param_with_default(
        "headers",
        parameters,
        context,
        StorageValue::Object(HashMap::new()),
    )?;

    let uri = Uri::try_from(&url).map_err(|e| ExecutionError::StateInvalid {
        message: format!("Failed to parse URL: {}", e),
        subject: OpenWebSocketClient::static_type().into(),
        inner: e.to_string(),
    })?;
    let mut request_builder = ClientRequestBuilder::new(uri);
    match headers_param {
        StorageValue::Object(object) => {
            for (key, value) in object {
                let value = value.to_string().map_err(ExecutionError::from)?;
                request_builder = request_builder.with_header(key, value);
            }
        }
        s => {
            return Err(ExecutionError::ParameterTypeMismatch {
                name: "headers".to_owned(),
                expected: Description::new_union(vec![
                    Description::new_base_type("array"),
                    Description::new_base_type("object"),
                ])
                .into(),
                actual: describe(s).into(),
            })
        }
    }
    let request =
        request_builder
            .into_client_request()
            .map_err(|e| ExecutionError::StateInvalid {
                message: format!("Failed to build WebSocket request: {}", e),
                subject: OpenWebSocketClient::static_type().into(),
                inner: e.to_string(),
            })?;

    // Connect to WebSocket
    let (ws_stream, _) =
        connect_async(request)
            .await
            .map_err(|e| ExecutionError::StateInvalid {
                message: format!("Failed to connect to WebSocket at {}", url),
                subject: OpenWebSocketClient::static_type().into(),
                inner: e.to_string(),
            })?;

    // Create command channel
    let (command_sender, command_receiver) = mpsc::channel::<WebSocketClientCommand>(32);

    // Create cancellation channel
    let (cancel_sender, cancel_receiver) = oneshot::channel::<()>();

    // Spawn background task to handle messages
    let global = context.global.clone();
    let actor_id = context.actor_id.clone();
    let url_clone = url.clone();
    tokio::spawn(async move {
        handle_websocket_messages(
            global,
            actor_id,
            url_clone,
            ws_stream,
            command_receiver,
            on_message_handler,
            on_close_handler,
            on_error_handler,
            cancel_receiver,
        )
        .await;
    });

    // Create resource
    let resource_id = generate_resource_id();
    let websocket_client = OpenWebSocketClient(resource_id, command_sender);
    context.resources.add_websocket_client(websocket_client);
    context
        .note_resource_provisioned(resource_id, OpenWebSocketClient::static_type())
        .await;

    // Create cancellation resource
    let cancellation_id = generate_resource_id();
    context.resources.add_cancellation(Cancellation {
        id: cancellation_id,
        sender: cancel_sender,
        name: cancellation_name(&url),
        subject: CancellationSubject::WebSocketClient,
    });
    context
        .note_resource_provisioned(cancellation_id, Cancellation::static_type())
        .await;

    Ok(())
}

#[instrument(level = "debug", skip_all)]
pub async fn send_message<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let message_param = eval_param("message", parameters, context)?;

    let message: TungsteniteMessage = match message_param {
        StorageValue::String(s) => TungsteniteMessage::Text(s.into()),
        StorageValue::Object(obj) => TungsteniteMessage::Text(
            serde_json::to_string(&obj)
                .map_err(|e| ExecutionError::StateInvalid {
                    message: "Failed to serialize object to JSON".to_owned(),
                    subject: OpenWebSocketClient::static_type().into(),
                    inner: e.to_string(),
                })?
                .into(),
        ),
        StorageValue::Array(arr) => {
            // Check if all elements are numbers (binary data)
            if arr.iter().all(|v| matches!(v, StorageValue::Number(_))) {
                let bytes: Result<Vec<u8>, _> = arr
                    .iter()
                    .map(|v| {
                        if let StorageValue::Number(n) = v {
                            Ok(*n as u8)
                        } else {
                            Err(ExecutionError::StateInvalid {
                                message: "Array must contain only numbers for binary message"
                                    .to_owned(),
                                subject: OpenWebSocketClient::static_type().into(),
                                inner: "Invalid array element".to_owned(),
                            })
                        }
                    })
                    .collect();
                TungsteniteMessage::Binary(Bytes::from(bytes?))
            } else {
                TungsteniteMessage::Text(
                    serde_json::to_string(&arr)
                        .map_err(|e| ExecutionError::StateInvalid {
                            message: "Failed to serialize array to JSON".to_owned(),
                            subject: OpenWebSocketClient::static_type().into(),
                            inner: e.to_string(),
                        })?
                        .into(),
                )
            }
        }
        v => TungsteniteMessage::Text(v.to_string().unwrap_or_else(|_| "null".to_owned()).into()),
    };

    let websocket_client = context
        .resources
        .use_websocket_client()
        .ok_or_else(OpenWebSocketClient::missing)?;

    let (command, receiver) = WebSocketClientCommand::new(message);
    websocket_client
        .1
        .send(command)
        .await
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to send message to WebSocket".to_owned(),
            subject: OpenWebSocketClient::static_type().to_owned(),
            inner: e.to_string(),
        })?;

    let resource_id = websocket_client.0;
    context
        .note_resource_used(resource_id, OpenWebSocketClient::static_type())
        .await;

    receiver.await.map_err(|e| ExecutionError::StateInvalid {
        message: "Failed to receive confirmation on WebSocket message".to_owned(),
        inner: e.to_string(),
        subject: OpenWebSocketClient::static_type().to_owned(),
    })?
}

#[instrument(level = "debug", skip_all)]
pub async fn close<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &[Parameter],
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let websocket_client = context
        .resources
        .pop_websocket_client()
        .ok_or_else(OpenWebSocketClient::missing)?;
    context
        .note_resource_consumed(websocket_client.0, websocket_client.get_type())
        .await;

    // Send close message
    let (command, receiver) = WebSocketClientCommand::new(TungsteniteMessage::Close(None));
    websocket_client
        .1
        .send(command)
        .await
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to send close frame to WebSocket".to_owned(),
            subject: OpenWebSocketClient::static_type().to_owned(),
            inner: e.to_string(),
        })?;

    let _ = receiver.await.map_err(|e| ExecutionError::StateInvalid {
        message: "Failed to receive confirmation on WebSocket closing message".to_owned(),
        inner: e.to_string(),
        subject: OpenWebSocketClient::static_type().to_owned(),
    })?;

    // Remove cancellation resource if exists
    // Note: We don't have the URL here, so we'll remove any WebSocketClient cancellation
    // This is a limitation, but should work for most cases
    if let Some(cancellation) = context
        .resources
        .pop_cancellation_where(|c| matches!(c.subject, CancellationSubject::WebSocketClient))
    {
        context
            .note_resource_consumed(cancellation.id, Cancellation::static_type())
            .await;
        let _ = cancellation.sender.send(());
    }

    Ok(())
}

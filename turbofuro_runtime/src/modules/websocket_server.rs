use axum::extract::ws::Message;
use tel::StorageValue;
use tokio::sync::{mpsc, oneshot};
use tracing::instrument;

use crate::{
    errors::ExecutionError,
    evaluations::{eval_param, eval_string_param, get_handlers_from_parameters},
    executor::{ExecutionContext, Parameter},
    modules::http_server::HttpRequestToRespond,
    resources::{HttpResponse, Resource, ResourceId},
};

pub const WEBSOCKET_RESOURCE_TYPE: &str = "websocket";

#[derive(Debug)]
pub struct WebSocketCommand {
    pub message: Message,
    pub responder: oneshot::Sender<Result<(), ExecutionError>>,
}

impl WebSocketCommand {
    pub fn new(message: Message) -> (Self, oneshot::Receiver<Result<(), ExecutionError>>) {
        let (responder, receiver) = oneshot::channel();
        let command = Self { message, responder };
        (command, receiver)
    }
}

#[derive(Debug)]
pub struct OpenWebSocket(pub ResourceId, pub mpsc::Sender<WebSocketCommand>);

impl Resource for OpenWebSocket {
    fn static_type() -> &'static str {
        WEBSOCKET_RESOURCE_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.0
    }
}

#[instrument(level = "debug", skip_all)]
pub async fn setup_route<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let path = eval_string_param("path", parameters, context)?;
    let handlers = get_handlers_from_parameters(parameters);
    {
        let mut router = context.global.registry.router.lock().await;
        router.add_route("get".into(), path, context.module.id.clone(), handlers)
    }

    Ok(())
}

#[instrument(level = "debug", skip_all)]
pub async fn accept_ws<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &[Parameter],
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let http_request_to_respond = context
        .resources
        .pop_http_request_to_respond()
        .ok_or_else(HttpRequestToRespond::missing)?;
    context
        .note_resource_consumed(
            http_request_to_respond.id,
            http_request_to_respond.get_type(),
        )
        .await;

    let (response, receiver) = HttpResponse::new_ws();
    http_request_to_respond
        .response_sender
        .send(response)
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to accept WebSocket by sending response to HTTP request".to_owned(),
            subject: HttpRequestToRespond::static_type().into(),
            inner: format!("{e:?}"),
        })?;

    receiver.await.map_err(|e| ExecutionError::StateInvalid {
        message: "Failed to receive confirmation on WebSocket acceptance".to_owned(),
        inner: e.to_string(),
        subject: HttpRequestToRespond::static_type().into(),
    })?
}

#[instrument(level = "debug", skip_all)]
pub async fn send_message<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let message_param = eval_param("message", parameters, context)?;

    let message: Message = match message_param {
        StorageValue::String(s) => Message::Text(s.into()),
        StorageValue::Object(obj) => Message::Text(serde_json::to_string(&obj).unwrap().into()),
        StorageValue::Array(arr) => Message::Text(serde_json::to_string(&arr).unwrap().into()),
        v => Message::Text(v.to_string().unwrap_or_default().into()),
    };

    let websocket = context
        .resources
        .use_websocket()
        .ok_or_else(OpenWebSocket::missing)?;

    let (command, receiver) = WebSocketCommand::new(message);
    websocket
        .1
        .send(command)
        .await
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to send message to WebSocket".to_owned(),
            subject: OpenWebSocket::static_type().to_owned(),
            inner: e.to_string(),
        })?;

    let resource_id = websocket.0;
    context
        .note_resource_used(resource_id, OpenWebSocket::static_type())
        .await;

    receiver.await.map_err(|e| ExecutionError::StateInvalid {
        message: "Failed to receive confirmation on WebSocket message".to_owned(),
        inner: e.to_string(),
        subject: OpenWebSocket::static_type().to_owned(),
    })?
}

#[instrument(level = "debug", skip_all)]
pub async fn close_websocket<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &[Parameter],
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let websocket = context
        .resources
        .pop_websocket()
        .ok_or_else(OpenWebSocket::missing)?;
    context
        .note_resource_consumed(websocket.0, websocket.get_type())
        .await;

    let (command, receiver) = WebSocketCommand::new(Message::Close(None));
    websocket
        .1
        .send(command)
        .await
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to send close frame to WebSocket".to_owned(),
            subject: OpenWebSocket::static_type().to_owned(),
            inner: e.to_string(),
        })?;

    receiver.await.map_err(|e| ExecutionError::StateInvalid {
        message: "Failed to receive confirmation on WebSocket closing message".to_owned(),
        inner: e.to_string(),
        subject: OpenWebSocket::static_type().to_owned(),
    })?
}

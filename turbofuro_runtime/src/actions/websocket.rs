use axum::extract::ws::Message;
use tel::StorageValue;
use tracing::instrument;

use crate::{
    errors::ExecutionError,
    evaluations::eval_param,
    executor::{ExecutionContext, Parameter},
    resources::{HttpRequestToRespond, HttpResponse, OpenWebSocket, Resource, WebSocketCommand},
};

use super::get_handlers_from_parameters;

#[instrument(level = "debug", skip_all)]
pub async fn setup_route<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let path_param = eval_param("path", parameters, &context.storage, &context.environment)?;
    let handlers = get_handlers_from_parameters(parameters);
    {
        let mut router = context.global.registry.router.lock().await;
        router.add_route(
            "get".into(),
            path_param.to_string()?,
            context.module.id.clone(),
            handlers,
        )
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
        .http_requests_to_respond
        .pop()
        .ok_or_else(HttpRequestToRespond::missing)?;

    let (response, receiver) = HttpResponse::new_ws();
    http_request_to_respond
        .response_sender
        .send(response)
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to accept WebSocket by sending response to HTTP request".to_owned(),
            subject: HttpRequestToRespond::get_type().into(),
            inner: format!("{:?}", e),
        })?;

    receiver.await.map_err(|e| ExecutionError::StateInvalid {
        message: "Failed to receive confirmation on WebSocket acceptance".to_owned(),
        inner: e.to_string(),
        subject: HttpRequestToRespond::get_type().into(),
    })?
}

#[instrument(level = "debug", skip_all)]
pub async fn send_message<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let message_param = eval_param(
        "message",
        parameters,
        &context.storage,
        &context.environment,
    )?;

    let message: Message = match message_param {
        StorageValue::String(s) => Message::Text(s),
        StorageValue::Object(obj) => Message::Text(serde_json::to_string(&obj).unwrap()),
        StorageValue::Array(arr) => Message::Text(serde_json::to_string(&arr).unwrap()),
        v => Message::Text(v.to_string().unwrap_or_default()),
    };

    let websocket = context
        .resources
        .websockets
        .first_mut()
        .ok_or_else(OpenWebSocket::missing)
        .map(|r| r.0.clone())?;

    let (command, receiver) = WebSocketCommand::new(message);
    websocket
        .send(command)
        .await
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to send message to WebSocket".to_owned(),
            subject: OpenWebSocket::get_type().to_owned(),
            inner: e.to_string(),
        })?;

    receiver.await.map_err(|e| ExecutionError::StateInvalid {
        message: "Failed to receive confirmation on WebSocket message".to_owned(),
        inner: e.to_string(),
        subject: OpenWebSocket::get_type().to_owned(),
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
        .websockets
        .first_mut()
        .ok_or_else(OpenWebSocket::missing)
        .map(|r| r.0.clone())?;

    let (command, receiver) = WebSocketCommand::new(Message::Close(None));
    websocket
        .send(command)
        .await
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to send close frame to WebSocket".to_owned(),
            subject: OpenWebSocket::get_type().to_owned(),
            inner: e.to_string(),
        })?;

    receiver.await.map_err(|e| ExecutionError::StateInvalid {
        message: "Failed to receive confirmation on WebSocket closing message".to_owned(),
        inner: e.to_string(),
        subject: OpenWebSocket::get_type().to_owned(),
    })?
}

use std::{collections::HashMap, convert::Infallible, time::Duration, vec};

use axum::{
    body::Body,
    response::{sse, Response},
};
use tel::{describe, Description, StorageValue};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, instrument, warn};

use crate::{
    actor::ActorCommand,
    errors::ExecutionError,
    evaluations::{eval_optional_param_with_default, eval_param},
    executor::{ExecutionContext, Parameter},
    resource_not_found,
    resources::{HttpRequestToRespond, HttpResponse, OpenSseStream, Resource},
};

use super::{as_integer, get_handlers_from_parameters};

#[instrument(level = "debug", skip_all)]
pub async fn setup_route<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let method_param = eval_optional_param_with_default(
        "method",
        parameters,
        &context.storage,
        &context.environment,
        "get".into(),
    )?;
    let path_param = eval_param("path", parameters, &context.storage, &context.environment)?;
    let handlers = get_handlers_from_parameters(parameters);
    {
        let mut router = context.global.registry.router.lock().await;
        router.add_route(
            method_param.to_string()?,
            path_param.to_string()?,
            context.module.id.clone(),
            handlers,
        )
    }

    Ok(())
}

#[instrument(level = "debug", skip_all)]
pub async fn setup_streaming_route<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let method_param = eval_optional_param_with_default(
        "method",
        parameters,
        &context.storage,
        &context.environment,
        "get".into(),
    )?;
    let path_param = eval_param("path", parameters, &context.storage, &context.environment)?;
    let handlers = get_handlers_from_parameters(parameters);
    {
        let mut router = context.global.registry.router.lock().await;
        router.add_streaming_route(
            method_param.to_string()?,
            path_param.to_string()?,
            context.module.id.clone(),
            handlers,
        )
    }

    Ok(())
}

#[instrument(level = "debug", skip_all)]
pub async fn respond_with<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let status_type_param = eval_optional_param_with_default(
        "status",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::Number(200.0),
    )?;
    let status = as_integer(status_type_param, "status")?;

    let body = eval_optional_param_with_default(
        "body",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::Null(None),
    )?;

    let mut response_builder = Response::builder().status(status as u16);

    // TODO: Make this more verbose and handle other types by specialized functions
    let response = {
        let body = match body {
            StorageValue::String(s) => {
                if s.starts_with("<!DOCTYPE html>") || s.starts_with("<html") {
                    response_builder =
                        response_builder.header("Content-Type", "text/html; charset=utf-8");
                } else {
                    response_builder =
                        response_builder.header("Content-Type", "text/plain; charset=utf-8");
                }
                Body::new(s)
            }
            StorageValue::Number(i) => {
                response_builder = response_builder.header("Content-Type", "application/json");
                let text = serde_json::to_string(&i).unwrap();
                Body::new(text)
            }
            StorageValue::Boolean(b) => {
                response_builder = response_builder.header("Content-Type", "application/json");
                let text = serde_json::to_string(&b).unwrap();
                Body::new(text)
            }
            StorageValue::Array(arr) => {
                response_builder = response_builder.header("Content-Type", "application/json");
                let text = serde_json::to_string(&arr).unwrap();
                Body::new(text)
            }
            StorageValue::Object(obj) => {
                response_builder = response_builder.header("Content-Type", "application/json");
                let text = serde_json::to_string(&obj).unwrap();
                Body::new(text)
            }
            StorageValue::Null(_) => Body::empty(),
        };

        let headers_param = eval_optional_param_with_default(
            "headers",
            parameters,
            &context.storage,
            &context.environment,
            StorageValue::Object(HashMap::new()),
        )?;
        match headers_param {
            StorageValue::Object(object) => {
                for (key, value) in object {
                    let value = value.to_string().map_err(ExecutionError::from)?;
                    response_builder = response_builder.header(key, value);
                }
            }
            s => {
                return Err(ExecutionError::ParameterTypeMismatch {
                    name: "headers".to_owned(),
                    expected: Description::new_union(vec![
                        Description::new_base_type("array"),
                        Description::new_base_type("object"),
                    ]),
                    actual: describe(s),
                })
            }
        }

        response_builder
            .body(body)
            .map_err(|_| ExecutionError::ParameterInvalid {
                name: "body".to_owned(),
                message: "Could not convert to string".to_owned(),
            })?
    };

    let http_request_to_respond = context
        .resources
        .http_requests_to_respond
        .pop()
        .ok_or(resource_not_found!(HttpRequestToRespond))?;

    let (response, receiver) = HttpResponse::new(response);
    http_request_to_respond
        .0
        .send(response)
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to respond to HTTP request".to_owned(),
            subject: HttpRequestToRespond::get_type().into(),
            inner: format!("{:?}", e),
        })?;

    receiver.await.map_err(|e| ExecutionError::StateInvalid {
        message: "Failed to receive confirmation on HTTP response".to_owned(),
        inner: e.to_string(),
        subject: HttpRequestToRespond::get_type().into(),
    })?
}

#[instrument(level = "debug", skip_all)]
pub async fn respond_with_stream<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let status_type_param = eval_optional_param_with_default(
        "status",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::Number(200.0),
    )?;
    let status = as_integer(status_type_param, "status")?;

    let mut response_builder = Response::builder().status(status as u16);

    // TODO: Make this more verbose and handle other types by specialized functions
    let response = {
        let stream = context.resources.get_nearest_stream()?;
        let body = Body::from_stream(stream);

        let headers_param = eval_optional_param_with_default(
            "headers",
            parameters,
            &context.storage,
            &context.environment,
            StorageValue::Object(HashMap::new()),
        )?;
        match headers_param {
            StorageValue::Object(object) => {
                for (key, value) in object {
                    let value = value.to_string().map_err(ExecutionError::from)?;
                    response_builder = response_builder.header(key, value);
                }
            }
            s => {
                return Err(ExecutionError::ParameterTypeMismatch {
                    name: "headers".to_owned(),
                    expected: Description::new_union(vec![
                        Description::new_base_type("array"),
                        Description::new_base_type("object"),
                    ]),
                    actual: describe(s),
                })
            }
        }

        response_builder
            .body(body)
            .map_err(|_| ExecutionError::ParameterInvalid {
                name: "body".to_owned(),
                message: "Could not build response".to_owned(),
            })?
    };

    let http_request_to_respond = context
        .resources
        .http_requests_to_respond
        .pop()
        .ok_or(resource_not_found!(HttpRequestToRespond))?;

    let (response, receiver) = HttpResponse::new(response);
    http_request_to_respond
        .0
        .send(response)
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to respond to HTTP request".to_owned(),
            subject: HttpRequestToRespond::get_type().into(),
            inner: format!("{:?}", e),
        })?;

    receiver.await.map_err(|e| ExecutionError::StateInvalid {
        message: "Failed to receive confirmation on HTTP response".to_owned(),
        inner: e.to_string(),
        subject: HttpRequestToRespond::get_type().into(),
    })?
}

#[instrument(level = "debug", skip_all)]
pub async fn respond_with_sse_stream<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let keep_alive_param = eval_optional_param_with_default(
        "keepAlive",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::Number(60.0),
    )?;

    let keep_alive = match keep_alive_param {
        StorageValue::Number(n) => Some(
            axum::response::sse::KeepAlive::new()
                .interval(Duration::from_secs(n as u64))
                .text("keep-alive"),
        ),
        _ => None,
    };

    let http_request_to_respond = context
        .resources
        .http_requests_to_respond
        .pop()
        .ok_or(resource_not_found!(HttpRequestToRespond))?;

    let (disconnect_sender, disconnect_receiver) = oneshot::channel::<StorageValue>();
    let actor_id = context.actor_id.clone();
    let global = context.global.clone();
    tokio::spawn(async move {
        match disconnect_receiver.await {
            Ok(_) => {
                let messenger = {
                    global
                        .registry
                        .actors
                        .get(&actor_id)
                        .map(|r| r.value().0.clone())
                };

                if let Some(messenger) = messenger {
                    messenger.send(ActorCommand::Terminate).await.expect(
                        "Failed to send terminate command to actor after SSE stream closed",
                    );
                } else {
                    warn!("SSE stream closed but actor {} was not found", actor_id);
                }
            }
            Err(_) => {
                // No-op
                debug!("SSE stream closed by server or unknown reason (?)");
            }
        }
    });

    let (event_sender, event_receiver) = mpsc::channel::<Result<sse::Event, Infallible>>(16);
    let (response, receiver) = HttpResponse::new_sse(event_receiver, keep_alive, disconnect_sender);
    http_request_to_respond
        .0
        .send(response)
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to respond to HTTP request".to_owned(),
            subject: HttpRequestToRespond::get_type().into(),
            inner: format!("{:?}", e),
        })?;

    context
        .resources
        .sse_streams
        .push(OpenSseStream(event_sender));

    receiver.await.map_err(|e| ExecutionError::StateInvalid {
        message: "Failed to receive confirmation on HTTP response".to_owned(),
        inner: e.to_string(),
        subject: HttpRequestToRespond::get_type().into(),
    })?
}

#[instrument(level = "debug", skip_all)]
pub async fn send_sse<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let event_param = eval_param("event", parameters, &context.storage, &context.environment)?;
    let event = match event_param {
        StorageValue::String(s) => sse::Event::default().data(s),
        StorageValue::Object(obj) => {
            sse::Event::default().data(serde_json::to_string(&obj).unwrap())
        }
        StorageValue::Array(arr) => {
            sse::Event::default().data(serde_json::to_string(&arr).unwrap())
        }
        v => sse::Event::default().data(v.to_string().unwrap_or_default()),
    };

    let sse_stream = context
        .resources
        .sse_streams
        .last_mut()
        .ok_or(resource_not_found!(OpenSseStream))?;

    sse_stream
        .0
        .send(Ok(event))
        .await
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to send SSE event".to_owned(),
            subject: OpenSseStream::get_type().into(),
            inner: e.to_string(),
        })?;

    Ok(())
}

#[instrument(level = "debug", skip_all)]
pub async fn close_sse_stream<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let sse_stream = context
        .resources
        .sse_streams
        .pop()
        .ok_or(resource_not_found!(OpenSseStream))?;

    drop(sse_stream);
    Ok(())
}

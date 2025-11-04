use std::{collections::HashMap, convert::Infallible, time::Duration, vec};

use axum::{
    body::Body,
    http::response,
    response::{sse, Response},
};
use axum_extra::extract::cookie::Cookie;
use cookie::time::OffsetDateTime;
use tel::{describe, Description, StorageValue};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, instrument};

use crate::{
    actor::ActorCommand,
    errors::ExecutionError,
    evaluations::{
        as_boolean, as_number, as_string, eval_opt_integer_param, eval_opt_string_param,
        eval_optional_param_with_default, eval_param, eval_string_param,
        get_handlers_from_parameters,
    },
    executor::{ExecutionContext, Parameter},
    resources::{
        generate_resource_id, HttpRequestToRespond, HttpResponse, OpenSseStream, Resource,
    },
};

#[instrument(level = "debug", skip_all)]
pub async fn setup_route<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store: Option<&str>,
) -> Result<(), ExecutionError> {
    let method_param =
        eval_opt_string_param("method", parameters, context)?.unwrap_or_else(|| "get".to_owned());
    let path_param = eval_string_param("path", parameters, context)?;
    let handlers = get_handlers_from_parameters(parameters);
    {
        let mut router = context.global.registry.router.lock().await;
        router.add_route(
            method_param,
            path_param,
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
    let method =
        eval_opt_string_param("method", parameters, context)?.unwrap_or_else(|| "get".to_owned());
    let path_param = eval_param("path", parameters, context)?;
    let handlers = get_handlers_from_parameters(parameters);
    {
        let mut router = context.global.registry.router.lock().await;
        router.add_streaming_route(
            method,
            path_param.to_string()?,
            context.module.id.clone(),
            handlers,
        )
    }

    Ok(())
}

fn fill_headers_response(
    mut response_builder: response::Builder,
    headers_param: StorageValue,
    cookies_param: StorageValue,
) -> Result<response::Builder, ExecutionError> {
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
                ])
                .into(),
                actual: describe(s).into(),
            })
        }
    }

    match cookies_param {
        StorageValue::Array(cookie_array) => {
            for (index, cookie) in cookie_array.into_iter().enumerate() {
                match cookie {
                    StorageValue::Object(cookie) => {
                        response_builder = apply_cookie(
                            cookie,
                            response_builder,
                            format!("cookies[{index}]").as_str(),
                        )?;
                    }
                    _ => {
                        return Err(ExecutionError::ParameterTypeMismatch {
                            name: "cookies".to_owned(),
                            expected: Description::new_base_type("object").into(),
                            actual: describe(cookie).into(),
                        })
                    }
                }
            }
        }
        StorageValue::Object(cookie_object) => {
            response_builder = apply_cookie(cookie_object, response_builder, "cookies")?;
        }
        s => {
            return Err(ExecutionError::ParameterTypeMismatch {
                name: "cookies".to_owned(),
                expected: Description::new_union(vec![
                    Description::new_base_type("array"),
                    Description::new_base_type("object"),
                ])
                .into(),
                actual: describe(s).into(),
            })
        }
    }

    Ok(response_builder)
}

#[instrument(level = "debug", skip_all)]
pub async fn respond_with<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let status = eval_opt_integer_param("status", parameters, context)?.unwrap_or(200);
    let headers_param = eval_optional_param_with_default(
        "headers",
        parameters,
        context,
        StorageValue::Object(HashMap::new()),
    )?;
    let cookies_param = eval_optional_param_with_default(
        "cookies",
        parameters,
        context,
        StorageValue::Array(vec![]),
    )?;

    let body =
        eval_optional_param_with_default("body", parameters, context, StorageValue::Null(None))?;

    let http_request_to_respond = context
        .resources
        .pop_http_request_to_respond()
        .ok_or_else(HttpRequestToRespond::missing)?;
    context
        .note_resource_consumed(
            http_request_to_respond.id,
            HttpRequestToRespond::static_type(),
        )
        .await;

    // TODO: Make this more verbose and handle other types by specialized functions
    let mut response_builder = Response::builder().status(status as u16);
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

        response_builder = fill_headers_response(response_builder, headers_param, cookies_param)?;

        response_builder
            .body(body)
            .map_err(|_| ExecutionError::ParameterInvalid {
                name: "body".to_owned(),
                message: "Could not convert to string".to_owned(),
            })?
    };

    let (response, receiver) = HttpResponse::new(response);
    http_request_to_respond
        .response_sender
        .send(response)
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to respond to HTTP request".to_owned(),
            subject: HttpRequestToRespond::static_type().into(),
            inner: format!("{e:?}"),
        })?;

    receiver.await.map_err(|e| ExecutionError::StateInvalid {
        message: "Failed to receive confirmation on HTTP response".to_owned(),
        inner: e.to_string(),
        subject: HttpRequestToRespond::static_type().into(),
    })?
}

fn apply_cookie(
    mut cookie: HashMap<String, StorageValue>,
    response_builder: response::Builder,
    path: &str,
) -> Result<response::Builder, ExecutionError> {
    let mut response_builder = response_builder;

    let name = as_string(
        cookie
            .remove("name")
            .ok_or_else(|| ExecutionError::ParameterInvalid {
                name: "name".to_owned(),
                message: "Cookie name is missing".to_owned(),
            })?,
        format!("{path}.name").as_str(),
    )?;
    let value = as_string(
        cookie
            .remove("value")
            .ok_or_else(|| ExecutionError::ParameterInvalid {
                name: "value".to_owned(),
                message: "Cookie name is missing".to_owned(),
            })?,
        format!("{path}.value").as_str(),
    )?;
    let expires = cookie
        .remove("expires")
        .map(|v| as_number(v, format!("{path}.expires").as_str()))
        .transpose()?;
    let max_age = cookie
        .remove("maxAge")
        .map(|v| as_number(v, format!("{path}.maxAge").as_str()))
        .transpose()?;
    let secure = cookie
        .remove("secure")
        .map(|v| as_boolean(v, format!("{path}.secure").as_str()))
        .transpose()?;
    let http_only = cookie
        .remove("httpOnly")
        .map(|v| as_boolean(v, format!("{path}.httpOnly").as_str()))
        .transpose()?;
    let same_site = cookie
        .remove("sameSite")
        .map(|v| as_string(v, format!("{path}.sameSite").as_str()))
        .transpose()?;

    let mut cookie_builder = Cookie::build((name, value));
    if let Some(expires) = expires {
        cookie_builder =
            cookie_builder.expires(OffsetDateTime::from_unix_timestamp(expires as i64).map_err(
                |e| ExecutionError::ParameterInvalid {
                    name: "expires".to_owned(),
                    message: e.to_string(),
                },
            )?);
    }
    if let Some(max_age) = max_age {
        cookie_builder = cookie_builder.max_age(cookie::time::Duration::seconds_f64(max_age));
    }
    if let Some(secure) = secure {
        cookie_builder = cookie_builder.secure(secure);
    }
    if let Some(http_only) = http_only {
        cookie_builder = cookie_builder.http_only(http_only);
    }
    if let Some(same_site) = same_site {
        match same_site.to_lowercase().as_str() {
            "lax" => cookie_builder = cookie_builder.same_site(cookie::SameSite::Lax),
            "strict" => cookie_builder = cookie_builder.same_site(cookie::SameSite::Strict),
            "none" => cookie_builder = cookie_builder.same_site(cookie::SameSite::None),
            _ => {
                return Err(ExecutionError::ParameterInvalid {
                    name: format!("{path}.sameSite"),
                    message: "Invalid value for sameSite".to_owned(),
                })
            }
        }
    }

    response_builder =
        response_builder.header("set-cookie", cookie_builder.build().encoded().to_string());

    Ok(response_builder)
}

#[instrument(level = "debug", skip_all)]
pub async fn respond_with_stream<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store: Option<&str>,
) -> Result<(), ExecutionError> {
    let status = eval_opt_integer_param("status", parameters, context)?.unwrap_or(200);
    let headers_param = eval_optional_param_with_default(
        "headers",
        parameters,
        context,
        StorageValue::Object(HashMap::new()),
    )?;
    let cookies_param = eval_optional_param_with_default(
        "cookies",
        parameters,
        context,
        StorageValue::Array(vec![]),
    )?;

    let mut response_builder = Response::builder().status(status as u16);
    let response = {
        let (stream, metadata) = context.resources.get_stream()?;
        context
            .note_resource_used(metadata.id, metadata.type_)
            .await;

        let body = Body::from_stream(stream);

        response_builder = fill_headers_response(response_builder, headers_param, cookies_param)?;
        response_builder
            .body(body)
            .map_err(|_| ExecutionError::ParameterInvalid {
                name: "body".to_owned(),
                message: "Could not build response".to_owned(),
            })?
    };

    let http_request_to_respond = context
        .resources
        .pop_http_request_to_respond()
        .ok_or_else(HttpRequestToRespond::missing)?;
    context
        .note_resource_consumed(
            http_request_to_respond.id,
            HttpRequestToRespond::static_type(),
        )
        .await;

    let (response, receiver) = HttpResponse::new(response);
    http_request_to_respond
        .response_sender
        .send(response)
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to respond to HTTP request".to_owned(),
            subject: HttpRequestToRespond::static_type().into(),
            inner: format!("{e:?}"),
        })?;

    receiver.await.map_err(|e| ExecutionError::StateInvalid {
        message: "Failed to receive confirmation on HTTP response".to_owned(),
        inner: e.to_string(),
        subject: HttpRequestToRespond::static_type().into(),
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
        context,
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
        .pop_http_request_to_respond()
        .ok_or_else(HttpRequestToRespond::missing)?;
    context
        .note_resource_consumed(
            http_request_to_respond.id,
            HttpRequestToRespond::static_type(),
        )
        .await;

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
                        .map(|r| r.value().clone())
                };

                if let Some(messenger) = messenger {
                    messenger.send(ActorCommand::Terminate).await.expect(
                        "Failed to send terminate command to actor after SSE stream closed",
                    );
                } else {
                    debug!("SSE stream closed but actor {} was not found. This might be normal if the actor was terminated and the stream is closed as the result of that.", actor_id);
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
        .response_sender
        .send(response)
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to respond to HTTP request".to_owned(),
            subject: HttpRequestToRespond::static_type().into(),
            inner: format!("{e:?}"),
        })?;

    let stream_id = generate_resource_id();
    context
        .resources
        .add_sse_stream(OpenSseStream(stream_id, event_sender));
    context
        .note_resource_provisioned(stream_id, OpenSseStream::static_type())
        .await;

    receiver.await.map_err(|e| ExecutionError::StateInvalid {
        message: "Failed to receive confirmation on HTTP response".to_owned(),
        inner: e.to_string(),
        subject: HttpRequestToRespond::static_type().into(),
    })?
}

#[instrument(level = "debug", skip_all)]
pub async fn send_sse<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let event_param = eval_param("event", parameters, context)?;
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
        .use_sse_stream()
        .ok_or_else(OpenSseStream::missing)?;

    sse_stream
        .1
        .send(Ok(event))
        .await
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to send SSE event".to_owned(),
            subject: OpenSseStream::static_type().into(),
            inner: e.to_string(),
        })?;

    let stream_id = sse_stream.0;
    context
        .note_resource_used(stream_id, OpenSseStream::static_type())
        .await;

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
        .pop_sse_stream()
        .ok_or_else(OpenSseStream::missing)?;
    context
        .note_resource_consumed(sse_stream.0, OpenSseStream::static_type())
        .await;

    drop(sse_stream);
    Ok(())
}

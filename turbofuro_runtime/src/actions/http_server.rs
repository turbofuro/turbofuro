use std::{collections::HashMap, vec};

use axum::{body::Body, response::Response};
use tel::{describe, Description, StorageValue};
use tokio_util::io::ReaderStream;
use tracing::instrument;

use crate::{
    errors::ExecutionError,
    evaluations::{eval_optional_param_with_default, eval_param},
    executor::{ExecutionContext, Parameter},
    resources::{HttpRequestToRespond, HttpResponse, Resource},
};

use super::{as_integer, get_handlers_from_parameters};

fn http_resource_not_found() -> ExecutionError {
    ExecutionError::MissingResource {
        resource_type: HttpRequestToRespond::get_type().into(),
    }
}

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
                    response_builder = response_builder.header("Content-Type", "text/html");
                } else {
                    response_builder = response_builder.header("Content-Type", "text/plain");
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
        .ok_or(http_resource_not_found())?;

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
pub async fn respond_with_file_stream<'a>(
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
        let file = context
            .resources
            .files
            .pop()
            .ok_or(ExecutionError::MissingResource {
                resource_type: HttpRequestToRespond::get_type().into(),
            })?;

        // convert the `AsyncRead` into a `Stream`
        let stream = ReaderStream::new(file.0);
        // convert the `Stream` into an `axum::body::HttpBody`
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
        .ok_or(http_resource_not_found())?;

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

use std::{collections::HashMap, time::Duration};

use http::{header, Method};
use mime::{Mime, TEXT_PLAIN};
use once_cell::sync::Lazy;
use reqwest::Client;
use tel::{describe, Description, ObjectBody, StorageValue};
use tracing::{debug, info, instrument};

use crate::{
    errors::ExecutionError,
    evaluations::{eval_optional_param, eval_optional_param_with_default, eval_param},
    executor::{ExecutionContext, Parameter},
    http_utils::decode_text_with_encoding,
    resources::{HttpRequestToRespond, Resource},
};

use super::{as_string, store_value};

static USER_AGENT: &str = concat!("turbofuro/", env!("CARGO_PKG_VERSION"));

static CLIENT: Lazy<Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap()
});

#[instrument(level = "trace", skip_all)]
pub async fn http_request<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let url_param = eval_param("url", parameters, &context.storage, &context.environment)?;
    let url = as_string(url_param, "url")?;

    let method_value = eval_optional_param_with_default(
        "method",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::String("get".to_string()),
    )?;
    let method = as_string(method_value, "method")?;

    let method = match method.to_lowercase().as_str() {
        "get" => Method::GET,
        "post" => Method::POST,
        "put" => Method::PUT,
        "head" => Method::HEAD,
        "options" => Method::OPTIONS,
        "patch" => Method::PATCH,
        "delete" => Method::DELETE,
        "trace" => Method::TRACE,
        method => {
            return Err(ExecutionError::ParameterInvalid {
                name: "method".to_string(),
                message: format!("Unknown method: {}", method),
            });
        }
    };

    let mut request_builder = CLIENT.request(method, url);

    if let Some(query) =
        eval_optional_param("query", parameters, &context.storage, &context.environment)?
    {
        request_builder = request_builder.query(&query);
    }

    let body_param =
        eval_optional_param("body", parameters, &context.storage, &context.environment)?;
    if let Some(body_param) = body_param {
        match body_param {
            StorageValue::String(data) => {
                request_builder = request_builder.body(data);
            }
            StorageValue::Number(data) => {
                request_builder = request_builder.body(data.to_string());
            }
            StorageValue::Boolean(data) => {
                request_builder = request_builder.body(data.to_string());
            }
            StorageValue::Array(arr) => {
                let serialized =
                    serde_json::to_string(&arr).map_err(|e| ExecutionError::ParameterInvalid {
                        name: "body".to_string(),
                        message: format!("Failed to serialize array: {}", e),
                    })?;
                request_builder = request_builder.body(serialized);
                request_builder = request_builder.header("Content-Type", "application/json");
            }
            StorageValue::Object(obj) => {
                let serialized =
                    serde_json::to_string(&obj).map_err(|e| ExecutionError::ParameterInvalid {
                        name: "body".to_string(),
                        message: format!("Failed to serialize object: {}", e),
                    })?;
                request_builder = request_builder.body(serialized);
                request_builder = request_builder.header("Content-Type", "application/json");
            }
            StorageValue::Null(_) => {
                // Do nothing
            }
        }
    }

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
                request_builder = request_builder.header(key, value);
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

    let request = request_builder
        .build()
        .map_err(|e| ExecutionError::StateInvalid {
            message: format!("Failed to build HTTP request: {}", e),
            subject: HttpRequestToRespond::get_type().into(),
            inner: e.to_string(),
        })?;

    let response = match CLIENT
        .execute(request)
        .await
        .map_err(|e| ExecutionError::StateInvalid {
            message: format!("Failed to execute HTTP request: {}", e),
            subject: HttpRequestToRespond::get_type().into(),
            inner: e.to_string(),
        }) {
        Ok(response) => response,
        Err(e) => {
            return Err(e);
        }
    };

    let mut response_object: ObjectBody = HashMap::new();
    response_object.insert(
        "status".to_string(),
        StorageValue::Number(response.status().as_u16().into()),
    );
    if let Some(reason) = response.status().canonical_reason() {
        response_object.insert(
            "statusText".to_string(),
            StorageValue::String(reason.to_string()),
        );
    }
    response_object.insert(
        "ok".to_string(),
        StorageValue::Boolean(response.status().is_success()),
    );

    let mut headers_object: ObjectBody = HashMap::new();
    for (k, v) in response.headers() {
        headers_object.insert(
            k.to_string(),
            StorageValue::String(v.to_str().unwrap_or_default().to_string()),
        );
    }
    response_object.insert("headers".to_string(), StorageValue::Object(headers_object));

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<Mime>().ok());

    let full = response.bytes().await.unwrap();
    let mime = content_type.unwrap_or(TEXT_PLAIN);
    let encoding_name = mime
        .get_param("charset")
        .map(|charset| charset.as_str())
        .unwrap_or("utf-8");

    let (text, replaced) = decode_text_with_encoding(encoding_name, &full);
    match replaced {
        true => {
            info!(
                "Unsupported mime type with encoding: {}, {}",
                mime, encoding_name
            );

            // Insert blob
            let vec = full
                .to_vec()
                .iter_mut()
                .map(|f| StorageValue::Number(*f as f64))
                .collect();

            response_object.insert("body".to_string(), StorageValue::Array(vec));
        }
        false => match serde_json::from_str::<StorageValue>(&text) {
            Ok(value) => {
                response_object.insert("body".to_string(), value);
                debug!("JSON response: {:?}", response_object);
            }
            Err(_) => {
                response_object.insert("body".to_string(), StorageValue::String(text));
                debug!("JSON parse errored");
            }
        },
    }

    store_value(
        store_as,
        context,
        step_id,
        StorageValue::Object(response_object),
    )
    .await?;
    Ok(())
}

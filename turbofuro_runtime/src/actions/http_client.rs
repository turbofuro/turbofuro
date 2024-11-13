use std::{collections::HashMap, time::Duration};

use axum::http::HeaderValue;
use hyper::{
    header::{self, CONTENT_TYPE},
    Method,
};
use mime::{Mime, TEXT_PLAIN};
use once_cell::sync::Lazy;
use reqwest::{Body, Certificate, Client, RequestBuilder, Response};
use tel::{describe, Description, ObjectBody, StorageValue};
use tracing::{info, instrument};

use crate::{
    errors::ExecutionError,
    evaluations::{
        as_string, eval_opt_boolean_param, eval_opt_string_param, eval_opt_u64_param,
        eval_optional_param, eval_optional_param_with_default, eval_string_param,
    },
    executor::{ExecutionContext, Parameter},
    http_utils::decode_text_with_encoding,
    resources::{
        FormDataDraft, HttpClient, HttpRequestToRespond, PendingHttpResponseBody, Resource,
    },
};

use super::store_value;

static DEFAULT_TIMEOUT: u64 = 60_000;

static USER_AGENT: &str = concat!("turbofuro/", env!("CARGO_PKG_VERSION"));

static CLIENT: Lazy<Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_millis(DEFAULT_TIMEOUT))
        .build()
        .unwrap()
});

fn get_builder(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
) -> Result<RequestBuilder, ExecutionError> {
    let url = eval_string_param("url", parameters, context)?;
    let method =
        eval_opt_string_param("method", parameters, context)?.unwrap_or_else(|| "get".to_owned());
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

    Ok(CLIENT.request(method, url))
}

#[derive(Debug)]
struct ResponseMetadata {
    ok: bool,
    status: u16,
    status_text: String,
    headers: HashMap<String, String>,
    content_type: Option<Mime>,
}

impl ResponseMetadata {
    fn into_storage_object(self) -> ObjectBody {
        let mut map: ObjectBody = HashMap::new();
        map.insert(
            "status".to_string(),
            StorageValue::Number(self.status.into()),
        );
        map.insert(
            "statusText".to_string(),
            StorageValue::String(self.status_text),
        );
        map.insert("ok".to_string(), StorageValue::Boolean(self.ok));
        map.insert(
            "headers".to_string(),
            StorageValue::Object(
                self.headers
                    .into_iter()
                    .map(|(k, v)| (k, StorageValue::String(v)))
                    .collect(),
            ),
        );
        map
    }
}

/// Flattens nested objects and arrays in the object
///
/// Example:
/// { items: [{ name: "John" }, { name: "Mary" }] }
///
/// Becomes:
/// items[0][name]=John
/// items[1][name]=Mary
fn prepare_deep_encoded(obj: ObjectBody) -> ObjectBody {
    let mut flatten = HashMap::new();
    for (key, value) in obj {
        match deep_encode(value) {
            StorageValue::Object(obj) => {
                for (sub_key, value) in obj {
                    flatten.insert(format!("{}{}", key, sub_key), value);
                }
            }
            v => {
                flatten.insert(key, v);
            }
        };
    }
    flatten
}

fn deep_encode(value: StorageValue) -> StorageValue {
    match value {
        StorageValue::Object(obj) => {
            let mut result = HashMap::new();
            for (key, value) in obj {
                let value = deep_encode(value);
                match value {
                    StorageValue::Object(obj) => {
                        for (sub_key, value) in obj {
                            result.insert(format!("[{}]{}", key, sub_key), value);
                        }
                    }
                    StorageValue::Array(arr) => {
                        for item in arr {
                            result.insert(format!("[{}]", key), item);
                        }
                    }
                    _ => {
                        result.insert(format!("[{}]", key), value);
                    }
                };
            }
            StorageValue::Object(result)
        }
        StorageValue::Array(arr) => {
            let mut result = HashMap::new();
            for (i, item) in arr.into_iter().enumerate() {
                let item = deep_encode(item);
                match item {
                    StorageValue::Object(obj) => {
                        for (key, value) in obj {
                            result.insert(format!("[{}]{}", i, key), value);
                        }
                    }
                    value => {
                        result.insert(format!("[{}]", i), value);
                    }
                };
            }
            StorageValue::Object(result)
        }
        v => v,
    }
}

fn get_metadata_object_from_response(response: &Response) -> ResponseMetadata {
    ResponseMetadata {
        ok: response.status().is_success(),
        status: response.status().as_u16(),
        status_text: response
            .status()
            .canonical_reason()
            .unwrap_or_default()
            .to_string(),
        headers: response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
            .collect(),
        content_type: response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<Mime>().ok()),
    }
}

fn set_static_body_from_parameters(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    mut request_builder: RequestBuilder,
) -> Result<RequestBuilder, ExecutionError> {
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
                    serde_json::to_vec(&arr).map_err(|e| ExecutionError::ParameterInvalid {
                        name: "body".to_string(),
                        message: format!("Failed to serialize array: {}", e),
                    })?;
                request_builder = request_builder.body(serialized);
                request_builder = request_builder
                    .header(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            }
            StorageValue::Object(obj) => {
                let serialized =
                    serde_json::to_vec(&obj).map_err(|e| ExecutionError::ParameterInvalid {
                        name: "body".to_string(),
                        message: format!("Failed to serialize object: {}", e),
                    })?;
                request_builder = request_builder.body(serialized);
                request_builder = request_builder
                    .header(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            }
            StorageValue::Null(_) => {
                // Do nothing
            }
        }
    }
    let form_param =
        eval_optional_param("form", parameters, &context.storage, &context.environment)?;
    if let Some(form_param) = form_param {
        match form_param {
            StorageValue::Object(obj) => {
                let serialized: String = serde_html_form::to_string(prepare_deep_encoded(obj))
                    .map_err(|e| ExecutionError::ParameterInvalid {
                        name: "form".to_string(),
                        message: format!("Failed to serialize object: {}", e),
                    })?;
                request_builder = request_builder.body(serialized);
                request_builder = request_builder.header(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/x-www-form-urlencoded"),
                );
            }
            StorageValue::Array(arr) => {
                let serialized = serde_urlencoded::to_string(arr).map_err(|e| {
                    ExecutionError::ParameterInvalid {
                        name: "form".to_string(),
                        message: format!("Failed to serialize array: {}", e),
                    }
                })?;
                request_builder = request_builder.body(serialized);
                request_builder = request_builder.header(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/x-www-form-urlencoded"),
                );
            }
            StorageValue::Null(_) => {
                // Do nothing
            }
            s => {
                return Err(ExecutionError::ParameterTypeMismatch {
                    name: "form".to_owned(),
                    expected: Description::new_base_type("object"),
                    actual: describe(s),
                })
            }
        }
    }
    Ok(request_builder)
}

async fn bare_http_request<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    mut request_builder: RequestBuilder,
) -> Result<Response, ExecutionError> {
    if let Some(query) =
        eval_optional_param("query", parameters, &context.storage, &context.environment)?
    {
        request_builder = request_builder.query(&query);
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
            message: "Failed to build HTTP request".to_owned(),
            subject: HttpRequestToRespond::get_type().into(),
            inner: e.to_string(),
        })?;

    let client: Client =
        match eval_optional_param("name", parameters, &context.storage, &context.environment)? {
            Some(param) => {
                let name = as_string(param, "name")?;
                context
                    .global
                    .registry
                    .http_clients
                    .get(&name)
                    .ok_or_else(HttpClient::missing)
                    .map(|r| r.value().clone().0)?
            }
            None => CLIENT.clone(),
        };

    let response = match client
        .execute(request)
        .await
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Failed to execute HTTP request".to_owned(),
            subject: HttpRequestToRespond::get_type().into(),
            inner: e.to_string(),
        }) {
        Ok(response) => response,
        Err(e) => {
            return Err(e);
        }
    };

    Ok(response)
}

async fn collect_body(
    response: Response,
    content_type: Option<Mime>,
) -> Result<StorageValue, ExecutionError> {
    let mime = content_type.unwrap_or(TEXT_PLAIN);
    let encoding_name = mime
        .get_param("charset")
        .map(|charset| charset.as_str())
        .unwrap_or("utf-8");

    let full = response.bytes().await.unwrap();
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

            Ok(StorageValue::Array(vec))
        }
        false => Ok(match serde_json::from_str::<StorageValue>(&text) {
            Ok(value) => value,
            Err(_) => StorageValue::String(text),
        }),
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn build_client<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let name = eval_string_param("name", parameters, context)?;
    let certificates = eval_optional_param_with_default(
        "rootCertificates",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::Array(vec![]),
    )?;
    let certificates = match certificates {
        StorageValue::Array(arr) => {
            let mut result = Vec::new();
            for cert in arr {
                let cert = as_string(cert, "rootCertificates")?;
                result.push(Certificate::from_pem(cert.as_bytes()).map_err(|e| {
                    ExecutionError::ParameterInvalid {
                        name: "rootCertificates".to_string(),
                        message: e.to_string(),
                    }
                })?);
            }
            Ok(result)
        }
        StorageValue::String(cert) => {
            let cert = Certificate::from_pem(cert.as_bytes()).map_err(|e| {
                ExecutionError::ParameterInvalid {
                    name: "rootCertificates".to_string(),
                    message: e.to_string(),
                }
            })?;
            Ok(vec![cert])
        }
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: "rootCertificates".to_string(),
            expected: Description::new_base_type("array"),
            actual: describe(s),
        }),
    }?;
    let accept_invalid_certificates =
        eval_opt_boolean_param("acceptInvalidCertificates", parameters, context)?.unwrap_or(false);
    let user_agent = eval_opt_string_param("userAgent", parameters, context)?
        .unwrap_or_else(|| USER_AGENT.to_owned());
    let timeout = eval_opt_u64_param("timeout", parameters, context)?.unwrap_or(DEFAULT_TIMEOUT);

    // TODO: Add support for default headers

    let mut builder = reqwest::Client::builder()
        .user_agent(user_agent)
        .danger_accept_invalid_certs(accept_invalid_certificates)
        .timeout(Duration::from_millis(timeout));

    for certificate in certificates {
        builder = builder.add_root_certificate(certificate);
    }

    let client = builder.build().map_err(|e| ExecutionError::StateInvalid {
        message: e.to_string(),
        subject: "http_client".into(),
        inner: "client".into(),
    })?;

    context
        .global
        .registry
        .http_clients
        .insert(name, HttpClient(client));

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn send_http_request<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let mut request_builder = get_builder(context, parameters)?;

    // Set body
    request_builder = set_static_body_from_parameters(context, parameters, request_builder)?;

    let response = bare_http_request(context, parameters, request_builder).await?;
    let metadata = get_metadata_object_from_response(&response);

    // Parse body
    let body = collect_body(response, metadata.content_type.clone()).await?;
    let mut response_object = metadata.into_storage_object();
    response_object.insert("body".to_string(), body);
    store_value(
        store_as,
        context,
        step_id,
        StorageValue::Object(response_object),
    )
    .await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn send_http_request_with_stream<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let mut request_builder = get_builder(context, parameters)?;

    // Pick a stream and set it as body
    let stream = context.resources.get_nearest_stream()?;
    request_builder = request_builder.body(Body::wrap_stream(stream));

    let response = bare_http_request(context, parameters, request_builder).await?;
    let metadata = get_metadata_object_from_response(&response);

    // Parse body
    let body = collect_body(response, metadata.content_type.clone()).await?;
    let mut response_object = metadata.into_storage_object();
    response_object.insert("body".to_string(), body);
    store_value(
        store_as,
        context,
        step_id,
        StorageValue::Object(response_object),
    )
    .await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn send_http_request_with_form_data<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let mut request_builder = get_builder(context, parameters)?;

    // Set body
    // Get form data from resources
    let form_data = context
        .resources
        .form_data
        .pop()
        .ok_or_else(FormDataDraft::missing)?;

    request_builder = request_builder.multipart(form_data.0);

    let response = bare_http_request(context, parameters, request_builder).await?;
    let metadata = get_metadata_object_from_response(&response);

    // Parse body
    let body = collect_body(response, metadata.content_type.clone()).await?;
    let mut response_object = metadata.into_storage_object();
    response_object.insert("body".to_string(), body);
    store_value(
        store_as,
        context,
        step_id,
        StorageValue::Object(response_object),
    )
    .await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn stream_http_request<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let mut request_builder = get_builder(context, parameters)?;

    // Set body
    request_builder = set_static_body_from_parameters(context, parameters, request_builder)?;

    let response = bare_http_request(context, parameters, request_builder).await?;
    let metadata = get_metadata_object_from_response(&response);

    // Put pending response
    context
        .resources
        .pending_response_body
        .push(PendingHttpResponseBody::new(response));

    store_value(
        store_as,
        context,
        step_id,
        StorageValue::Object(metadata.into_storage_object()),
    )
    .await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn stream_http_request_with_stream<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let mut request_builder = get_builder(context, parameters)?;

    // Pick a stream and set it as body
    let stream = context.resources.get_nearest_stream()?;
    request_builder = request_builder.body(Body::wrap_stream(stream));

    let response = bare_http_request(context, parameters, request_builder).await?;
    let metadata = get_metadata_object_from_response(&response);

    // Put pending response
    context
        .resources
        .pending_response_body
        .push(PendingHttpResponseBody::new(response));

    store_value(
        store_as,
        context,
        step_id,
        StorageValue::Object(metadata.into_storage_object()),
    )
    .await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn stream_http_request_with_form_data<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let mut request_builder = get_builder(context, parameters)?;

    // Set body
    // Get form data from resources
    let form_data = context
        .resources
        .form_data
        .pop()
        .ok_or_else(FormDataDraft::missing)?;

    request_builder = request_builder.multipart(form_data.0);

    let response = bare_http_request(context, parameters, request_builder).await?;
    let metadata = get_metadata_object_from_response(&response);

    // Put pending response
    context
        .resources
        .pending_response_body
        .push(PendingHttpResponseBody::new(response));

    store_value(
        store_as,
        context,
        step_id,
        StorageValue::Object(metadata.into_storage_object()),
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod test_http_client {
    use serde_json::json;
    use tel::NULL;

    use super::*;

    #[test]
    fn test_deep_encode_ignore_primitives() {
        assert_eq!(deep_encode("Hello World".into()), "Hello World".into());
        assert_eq!(deep_encode(5.into()), 5.into());
        assert_eq!(deep_encode(true.into()), true.into());
        assert_eq!(deep_encode(NULL), NULL);
    }

    #[test]
    fn test_deep_encode() {
        let input: ObjectBody = serde_json::from_value(json!({
            "items": [
                {
                    "name": "John",
                    "age": 30
                },
                {
                    "name": "Mary",
                    "age": 25
                }
            ],
            "message": "Hello World",
            "details": {
                "name": "John",
                "age": 30
            }
        }))
        .unwrap();

        let expected: ObjectBody = serde_json::from_value(json!({
            "items[0][name]": "John",
            "items[0][age]": 30,
            "items[1][name]": "Mary",
            "items[1][age]": 25,
            "message": "Hello World",
            "details[name]": "John",
            "details[age]": 30
        }))
        .unwrap();

        assert_eq!(prepare_deep_encoded(input), expected);
    }
}

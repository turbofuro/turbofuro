use std::collections::HashMap;

use tel::{StorageValue, NULL};
use tracing::instrument;
use url::Url;

use crate::{
    errors::ExecutionError,
    evaluations::{
        eval_opt_boolean_param, eval_opt_string_param, eval_optional_param, eval_param,
        eval_string_param,
    },
    executor::{ExecutionContext, Parameter},
};

use super::store_value;

#[instrument(level = "trace", skip_all)]
pub async fn parse_json(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let json = eval_string_param("json", parameters, context)?;

    let parsed: StorageValue =
        serde_json::from_str(&json).map_err(|e| ExecutionError::SerializationFailed {
            message: "Failed to parse JSON".to_owned(),
            breadcrumbs: "action/parse_json".to_string(),
            inner: e.to_string(),
        })?;

    store_value(store_as, context, step_id, parsed).await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn to_json(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let value_param = eval_param("value", parameters, context)?;
    let pretty = eval_opt_boolean_param("pretty", parameters, context)?.unwrap_or(false);

    let json = if pretty {
        serde_json::to_string_pretty(&value_param).map_err(|e| {
            ExecutionError::SerializationFailed {
                message: "Failed to serialize to JSON".to_owned(),
                breadcrumbs: "action/to_json".to_string(),
                inner: e.to_string(),
            }
        })?
    } else {
        serde_json::to_string(&value_param).map_err(|e| ExecutionError::SerializationFailed {
            message: "Failed to serialize to JSON".to_owned(),
            breadcrumbs: "action/to_json".to_string(),
            inner: e.to_string(),
        })?
    };

    store_value(store_as, context, step_id, json.into()).await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn parse_urlencoded(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let urlencoded = eval_string_param("urlencoded", parameters, context)?;

    let parsed: StorageValue = serde_urlencoded::from_str(&urlencoded).map_err(|e| {
        ExecutionError::SerializationFailed {
            message: "Failed to parse URL encoded".to_owned(),
            breadcrumbs: "action/parse_urlencoded".to_string(),
            inner: e.to_string(),
        }
    })?;

    store_value(store_as, context, step_id, parsed).await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn to_urlencoded(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let value_param = eval_param("value", parameters, context)?;

    let json = serde_urlencoded::to_string(value_param).map_err(|e| {
        ExecutionError::SerializationFailed {
            message: "Failed to serialize to URL encoded".to_owned(),
            breadcrumbs: "action/to_urlencoded".to_string(),
            inner: e.to_string(),
        }
    })?;

    store_value(store_as, context, step_id, json.into()).await?;
    Ok(())
}

/**
 * https://url.spec.whatwg.org/#url-parsing
 */
#[instrument(level = "trace", skip_all)]
pub async fn parse_url(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let url = eval_string_param("url", parameters, context)?;

    let parsed = Url::parse(&url).map_err(|e| ExecutionError::ParameterInvalid {
        name: "url".into(),
        message: e.to_string(),
    })?;

    let origin: StorageValue = parsed.origin().ascii_serialization().into();
    let host: StorageValue = parsed.host_str().map(|host| host.into()).unwrap_or(NULL);
    let path: StorageValue = parsed.path().into();
    let query: StorageValue = parsed.query().map(|query| query.into()).unwrap_or(NULL);
    let fragment: StorageValue = parsed
        .fragment()
        .map(|fragment| fragment.into())
        .unwrap_or(NULL);
    let scheme: StorageValue = parsed.scheme().into();
    let password: StorageValue = parsed
        .password()
        .map(|password| password.into())
        .unwrap_or(NULL);
    let username: StorageValue = {
        let username = parsed.username();
        if username.is_empty() {
            NULL
        } else {
            username.into()
        }
    };
    let port: StorageValue = parsed
        .port()
        .map(|port| StorageValue::Number(port.into()))
        .unwrap_or(NULL);

    let mut output: HashMap<String, StorageValue> = HashMap::new();
    output.insert("host".into(), host);
    output.insert("origin".into(), origin);
    output.insert("path".into(), path);
    output.insert("query".into(), query);
    output.insert("fragment".into(), fragment);
    output.insert("scheme".into(), scheme);
    output.insert("password".into(), password);
    output.insert("username".into(), username);
    output.insert("port".into(), port);

    store_value(store_as, context, step_id, StorageValue::Object(output)).await?;

    Ok(())
}

/**
 * https://url.spec.whatwg.org/#url-serializing
 */
#[instrument(level = "trace", skip_all)]
pub async fn to_url(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let host = eval_string_param("host", parameters, context)?;
    let scheme = eval_opt_string_param("scheme", parameters, context)?.unwrap_or("https".into());
    let port_param = eval_optional_param("port", parameters, context)?;

    let path = eval_opt_string_param("path", parameters, context)?;
    let query = eval_opt_string_param("query", parameters, context)?;
    let fragment = eval_opt_string_param("fragment", parameters, context)?;
    let username = eval_opt_string_param("username", parameters, context)?;
    let password = eval_opt_string_param("password", parameters, context)?;

    // Construct the URL
    let mut url = Url::parse(&format!("{}://{}", scheme, host)).map_err(|e| {
        ExecutionError::ParameterInvalid {
            name: "host".into(),
            message: e.to_string(),
        }
    })?;

    if let Some(port_value) = port_param {
        let port = match port_value {
            StorageValue::Number(port) => {
                u16::try_from(port as i64).map_err(|_e| ExecutionError::ParameterInvalid {
                    name: "port".into(),
                    message: "Port could not be parsed".into(),
                })?
            }
            StorageValue::String(port) => {
                port.parse::<u16>()
                    .map_err(|_e| ExecutionError::ParameterInvalid {
                        name: "port".into(),
                        message: "Port could not be parsed".into(),
                    })?
            }
            _ => {
                return Err(ExecutionError::ParameterInvalid {
                    name: "port".into(),
                    message: "Port must be a number between 0 and".into(),
                });
            }
        };
        url.set_port(Some(port))
            .map_err(|_e| ExecutionError::StateInvalid {
                message: "Port could not be set".into(),
                subject: "url".into(),
                inner: "port".into(), // TODO: Errors
            })?;
    }

    if let Some(path) = path {
        url.set_path(&path);
    }

    if let Some(query) = query {
        url.set_query(Some(&query));
    }

    if let Some(fragment) = fragment {
        url.set_fragment(Some(&fragment));
    }

    if let Some(username) = username {
        url.set_username(&username)
            .map_err(|_e| ExecutionError::StateInvalid {
                message: "Username could not be set".into(),
                subject: "url".into(),
                inner: "username".into(), // TODO: Errors
            })?;
    }

    if let Some(password) = password {
        url.set_password(Some(&password))
            .map_err(|_e| ExecutionError::StateInvalid {
                message: "Password could not be set".into(),
                subject: "url".into(),
                inner: "password".into(), // TODO: Errors
            })?;
    }

    store_value(store_as, context, step_id, StorageValue::String(url.into())).await?;

    Ok(())
}

#[cfg(test)]
mod test_convert {
    use crate::{
        evaluations::eval,
        executor::{ExecutionEvent, ExecutionTest},
    };

    use super::*;

    #[tokio::test]
    async fn test_parse_json() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        let result = parse_json(
            &mut context,
            &vec![Parameter::tel("json", r#""{\"test\":\"Hello World\"}""#)],
            "test",
            Some("obj"),
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(
            eval("obj.test", &context.storage, &context.environment).unwrap(),
            StorageValue::String("Hello World".to_owned())
        );
        assert!(context
            .log
            .events
            .iter()
            .any(|e| matches!(e, ExecutionEvent::StorageUpdated { id, selector: _, value: _ } if id == "test")));
    }

    #[tokio::test]
    async fn test_to_json() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        let result = to_json(
            &mut context,
            &vec![Parameter::tel("value", "{ message: \"Hello World\" }")],
            "test",
            Some("json"),
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(
            eval("json", &context.storage, &context.environment).unwrap(),
            StorageValue::String("{\"message\":\"Hello World\"}".to_owned())
        );
    }

    #[tokio::test]
    async fn test_parse_urlencoded() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        parse_urlencoded(
            &mut context,
            &vec![Parameter::tel("urlencoded", "\"message=Hello+World\"")],
            "test",
            Some("obj"),
        )
        .await
        .unwrap();

        assert_eq!(
            eval("obj.message", &context.storage, &context.environment).unwrap(),
            StorageValue::String("Hello World".to_owned())
        );
        assert!(context
            .log
            .events
            .iter()
            .any(|e| matches!(e, ExecutionEvent::StorageUpdated { id, selector: _, value: _ } if id == "test")));
    }

    #[tokio::test]
    async fn test_to_urlencoded() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        to_urlencoded(
            &mut context,
            &vec![Parameter::tel("value", "{ message: \"Hello World\" }")],
            "test",
            Some("data"),
        )
        .await
        .unwrap();

        assert_eq!(
            eval("data", &context.storage, &context.environment).unwrap(),
            StorageValue::String("message=Hello+World".to_owned())
        );
    }

    #[tokio::test]
    async fn test_parse_basic() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        parse_url(
            &mut context,
            &vec![Parameter::tel(
                "url",
                "\"https://example.com/test?query=1#fragment\"",
            )],
            "test",
            Some("parsed"),
        )
        .await
        .unwrap();

        let parsed = eval("parsed", &context.storage, &context.environment).unwrap();
        assert_eq!(
            parsed,
            StorageValue::Object(
                vec![
                    ("origin".to_owned(), "https://example.com".into()),
                    ("host".to_owned(), "example.com".into()),
                    ("path".to_owned(), "/test".into()),
                    ("query".to_owned(), "query=1".into()),
                    ("fragment".to_owned(), "fragment".into()),
                    ("scheme".to_owned(), "https".into()),
                    ("password".to_owned(), NULL),
                    ("username".to_owned(), NULL),
                    ("port".to_owned(), NULL),
                ]
                .into_iter()
                .collect()
            )
        );
    }

    #[tokio::test]
    async fn test_parse_complex() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        parse_url(
            &mut context,
            &vec![Parameter::tel(
                "url",
                "\"redis://username:pass@localhost:6379/wow\"",
            )],
            "test",
            Some("parsed"),
        )
        .await
        .unwrap();

        let parsed = eval("parsed", &context.storage, &context.environment).unwrap();
        assert_eq!(
            parsed,
            StorageValue::Object(
                vec![
                    ("origin".to_owned(), "null".into()),
                    ("host".to_owned(), "localhost".into()),
                    ("path".to_owned(), StorageValue::String("/wow".to_owned())),
                    ("query".to_owned(), NULL),
                    ("fragment".to_owned(), NULL),
                    ("scheme".to_owned(), "redis".into()),
                    ("password".to_owned(), "pass".into()),
                    ("username".to_owned(), "username".into()),
                    ("port".to_owned(), StorageValue::Number(6379.0)),
                ]
                .into_iter()
                .collect()
            )
        );
    }

    #[tokio::test]
    async fn test_to_url() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        to_url(
            &mut context,
            &vec![
                Parameter::tel("host", "\"example.org\""),
                Parameter::tel("path", "\"/test\""),
                Parameter::tel("query", "\"query=1\""),
                Parameter::tel("fragment", "\"fragment\""),
            ],
            "test",
            Some("url"),
        )
        .await
        .unwrap();

        let parsed = eval("url", &context.storage, &context.environment).unwrap();
        assert_eq!(
            parsed,
            StorageValue::String("https://example.org/test?query=1#fragment".to_owned())
        );
    }

    #[tokio::test]
    async fn test_to_url_complex() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        to_url(
            &mut context,
            &vec![
                Parameter::tel("host", "\"localhost\""),
                Parameter::tel("scheme", "\"redis\""),
                Parameter::tel("path", "\"/wow\""),
                Parameter::tel("username", "\"username\""),
                Parameter::tel("password", "\"pass\""),
                Parameter::tel("port", "6379"),
            ],
            "test",
            Some("url"),
        )
        .await
        .unwrap();

        let parsed = eval("url", &context.storage, &context.environment).unwrap();
        assert_eq!(
            parsed,
            StorageValue::String("redis://username:pass@localhost:6379/wow".to_owned())
        );
    }
}

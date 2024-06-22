use std::collections::HashMap;

use tel::{StorageValue, NULL};
use tracing::instrument;
use url::Url;

use crate::{
    errors::ExecutionError,
    evaluations::{eval_optional_param, eval_optional_param_with_default, eval_param},
    executor::{ExecutionContext, Parameter},
};

use super::{as_string, store_value};

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
    let url_param = eval_param("url", parameters, &context.storage, &context.environment)?;
    let url = as_string(url_param, "url")?;

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
    let username: StorageValue = parsed.username().into(); // TODO: Why is username not an Option?
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
pub async fn serialize_url(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let host_param = eval_param("host", parameters, &context.storage, &context.environment)?;
    let host = as_string(host_param, "host")?;

    let scheme_param: StorageValue = eval_optional_param_with_default(
        "scheme",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::String("https".into()),
    )?;
    let scheme = as_string(scheme_param, "scheme")?;
    let port_param =
        eval_optional_param("port", parameters, &context.storage, &context.environment)?;

    let path_param =
        eval_optional_param("path", parameters, &context.storage, &context.environment)?;
    let query_param =
        eval_optional_param("query", parameters, &context.storage, &context.environment)?;
    let fragment = eval_optional_param(
        "fragment",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    let username = eval_optional_param(
        "username",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    let password = eval_optional_param(
        "password",
        parameters,
        &context.storage,
        &context.environment,
    )?;

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

    if let Some(path) = path_param {
        let path = as_string(path, "path")?;
        url.set_path(&path);
    }

    if let Some(query) = query_param {
        let query = as_string(query, "query")?;
        url.set_query(Some(&query));
    }

    if let Some(fragment) = fragment {
        let fragment = as_string(fragment, "fragment")?;
        url.set_fragment(Some(&fragment));
    }

    if let Some(username) = username {
        let username = as_string(username, "username")?;
        url.set_username(&username)
            .map_err(|_e| ExecutionError::StateInvalid {
                message: "Username could not be set".into(),
                subject: "url".into(),
                inner: "username".into(), // TODO: Errors
            })?;
    }

    if let Some(password) = password {
        let password = as_string(password, "password")?;
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
mod test_url {
    use crate::{evaluations::eval, executor::ExecutionTest};

    use super::*;

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
                    (
                        "query".to_owned(),
                        "query=1".into() // TODO: Should this be a hashmap?
                    ),
                    ("fragment".to_owned(), "fragment".into()),
                    ("scheme".to_owned(), "https".into()),
                    ("password".to_owned(), NULL),
                    ("username".to_owned(), "".into()),
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
    async fn test_serialize() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        serialize_url(
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
    async fn test_serialize_complex() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        serialize_url(
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
use tel::StorageValue;
use tracing::instrument;

use crate::{
    errors::ExecutionError,
    evaluations::eval_param,
    executor::{ExecutionContext, Parameter},
};

use super::{as_string, store_value};

#[instrument(level = "trace", skip_all)]
pub fn parse_json(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let json = eval_param("json", parameters, &context.storage, &context.environment)?;
    let json = as_string(json, "json")?;

    let parsed: StorageValue =
        serde_json::from_str(&json).map_err(|e| ExecutionError::SerializationFailed {
            message: "Failed to parse JSON".to_owned(),
            breadcrumbs: "action/parse_json".to_string(),
            inner: e.to_string(),
        })?;

    store_value(store_as, context, step_id, parsed)?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub fn to_json(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let value_param = eval_param("value", parameters, &context.storage, &context.environment)?;

    let json =
        serde_json::to_string(&value_param).map_err(|e| ExecutionError::SerializationFailed {
            message: "Failed to serialize to JSON".to_owned(),
            breadcrumbs: "action/to_json".to_string(),
            inner: e.to_string(),
        })?;

    store_value(store_as, context, step_id, json.into())?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub fn parse_urlencoded(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let urlencoded = eval_param(
        "urlencoded",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    let urlencoded = as_string(urlencoded, "urlencoded")?;

    let parsed: StorageValue = serde_urlencoded::from_str(&urlencoded).map_err(|e| {
        ExecutionError::SerializationFailed {
            message: "Failed to parse URL encoded".to_owned(),
            breadcrumbs: "action/parse_urlencoded".to_string(),
            inner: e.to_string(),
        }
    })?;

    store_value(store_as, context, step_id, parsed)?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub fn to_urlencoded(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let value_param = eval_param("value", parameters, &context.storage, &context.environment)?;

    let json = serde_urlencoded::to_string(value_param).map_err(|e| {
        ExecutionError::SerializationFailed {
            message: "Failed to serialize to URL encoded".to_owned(),
            breadcrumbs: "action/to_urlencoded".to_string(),
            inner: e.to_string(),
        }
    })?;

    store_value(store_as, context, step_id, json.into())?;
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
        );

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
        );

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
        .unwrap();

        assert_eq!(
            eval("data", &context.storage, &context.environment).unwrap(),
            StorageValue::String("message=Hello+World".to_owned())
        );
    }
}

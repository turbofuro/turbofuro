use std::sync::Arc;

use dashmap::DashMap;
use once_cell::sync::Lazy;
use tel::StorageValue;
use tracing::instrument;

use crate::{
    errors::ExecutionError,
    evaluations::{eval_optional_param, eval_optional_param_with_default, eval_param},
    executor::{get_timestamp, ExecutionContext, Parameter},
};

use super::{as_number, as_u64, store_value};

#[derive(Debug, Clone)]
struct KvValue {
    value: StorageValue,
    expiration: Option<u64>,
}

impl From<StorageValue> for KvValue {
    fn from(value: StorageValue) -> Self {
        KvValue {
            value,
            expiration: None,
        }
    }
}

static KV: Lazy<Arc<DashMap<String, KvValue>>> = Lazy::new(|| Arc::new(DashMap::new()));

#[instrument(level = "trace", skip_all)]
pub async fn read_from_store<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let key_param = eval_param("key", parameters, &context.storage, &context.environment)?;
    let key = key_param.to_string().map_err(ExecutionError::from)?;

    let kv_value = KV
        .get(key.as_str())
        .map(|v| v.clone())
        .unwrap_or(StorageValue::Null(None).into());

    if let Some(expiration) = kv_value.expiration {
        let now = get_timestamp();
        if now > expiration {
            KV.remove(key.as_str());
            store_value(store_as, context, step_id, StorageValue::Null(None)).await?;
            return Ok(());
        }
    }

    store_value(store_as, context, step_id, kv_value.value).await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn write_to_store<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let key_param = eval_param("key", parameters, &context.storage, &context.environment)?;
    let key = key_param.to_string().map_err(ExecutionError::from)?;

    let value = eval_param("value", parameters, &context.storage, &context.environment)?;

    let mut expiration = None;
    let expiration_param = eval_optional_param(
        "expiration",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    if let Some(expiration_param) = expiration_param {
        expiration = Some(get_timestamp() + as_u64(expiration_param, "expiration")?);
    }

    KV.insert(key, KvValue { value, expiration });

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn increment_store<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let key_param = eval_param("key", parameters, &context.storage, &context.environment)?;
    let key = key_param.to_string().map_err(ExecutionError::from)?;

    let value = eval_optional_param_with_default(
        "increment",
        parameters,
        &context.storage,
        &context.environment,
        1.into(),
    )?;
    let increment = as_number(value, "increment")?;

    let mut expiration = None;
    let expiration_param = eval_optional_param(
        "expiration",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    if let Some(expiration_param) = expiration_param {
        expiration = Some(get_timestamp() + as_u64(expiration_param, "expiration")?);
    }

    if let Some(mut existing) = KV.get_mut(key.as_str()) {
        existing.expiration = expiration;
        existing.value = match existing.value {
            StorageValue::Number(n) => StorageValue::Number(n + increment),
            _ => {
                return Err(ExecutionError::StateInvalid {
                    message: "Can't increment non-number".to_owned(),
                    subject: "kv".to_owned(),
                    inner: "increment".to_owned(),
                })
            }
        };
    } else {
        KV.insert(
            key,
            KvValue {
                value: increment.into(),
                expiration,
            },
        );
    }

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn delete_from_store<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let key_param = eval_param("key", parameters, &context.storage, &context.environment)?;
    let key = key_param.to_string().map_err(ExecutionError::from)?;

    KV.remove(key.as_str());

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::executor::ExecutionTest;

    use super::*;

    #[tokio::test]
    async fn test_read_write_delete() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        write_to_store(
            &mut context,
            &vec![
                Parameter::tel("key", "\"test_key\""),
                Parameter::tel("value", "12"),
            ],
            "test",
        )
        .await
        .unwrap();

        read_from_store(
            &mut context,
            &vec![Parameter::tel("key", "\"test_key\"")],
            "test",
            Some("data"),
        )
        .await
        .unwrap();

        assert_eq!(
            context.storage.get("data"),
            Some(&StorageValue::Number(12.0))
        );

        delete_from_store(
            &mut context,
            &vec![Parameter::Tel {
                name: "key".to_owned(),
                expression: "\"test_key\"".to_owned(),
            }],
            "test",
        )
        .await
        .unwrap();

        read_from_store(
            &mut context,
            &vec![Parameter::tel("key", "\"test_key\"")],
            "test",
            Some("data"),
        )
        .await
        .unwrap();

        assert_eq!(context.storage.get("data"), Some(&StorageValue::Null(None)));
    }

    #[tokio::test]
    async fn test_expiration() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        write_to_store(
            &mut context,
            &vec![
                Parameter::tel("key", "\"test_key\""),
                Parameter::tel("value", "4"),
                Parameter::tel("expiration", "100"),
            ],
            "test",
        )
        .await
        .unwrap();

        read_from_store(
            &mut context,
            &vec![Parameter::tel("key", "\"test_key\"")],
            "test",
            Some("data"),
        )
        .await
        .unwrap();
        assert_eq!(
            context.storage.get("data"),
            Some(&StorageValue::Number(4.0))
        );

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        read_from_store(
            &mut context,
            &vec![Parameter::tel("key", "\"test_key\"")],
            "test",
            Some("data"),
        )
        .await
        .unwrap();

        assert_eq!(context.storage.get("data"), Some(&StorageValue::Null(None)));
    }

    #[tokio::test]
    async fn test_increment() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        increment_store(
            &mut context,
            &vec![
                Parameter::tel("key", "\"test_key\""),
                Parameter::tel("increment", "2"),
            ],
            "test",
        )
        .await
        .unwrap();

        read_from_store(
            &mut context,
            &vec![Parameter::tel("key", "\"test_key\"")],
            "test",
            Some("data"),
        )
        .await
        .unwrap();
        assert_eq!(
            context.storage.get("data"),
            Some(&StorageValue::Number(2.0))
        );

        increment_store(
            &mut context,
            &vec![
                Parameter::tel("key", "\"test_key\""),
                Parameter::tel("increment", "3"),
            ],
            "test",
        )
        .await
        .unwrap();

        read_from_store(
            &mut context,
            &vec![Parameter::tel("key", "\"test_key\"")],
            "test",
            Some("data"),
        )
        .await
        .unwrap();
        assert_eq!(
            context.storage.get("data"),
            Some(&StorageValue::Number(5.0))
        );
    }
}

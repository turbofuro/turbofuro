use std::sync::{atomic::AtomicBool, Arc};

use dashmap::DashMap;
use once_cell::sync::Lazy;
use tel::StorageValue;
use tracing::{instrument, warn};

use crate::{
    errors::ExecutionError,
    evaluations::{eval_opt_number_param, eval_opt_u64_param, eval_param, eval_string_param},
    executor::{get_timestamp, ExecutionContext, Parameter},
};

use super::store_value;

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
static CLEANER_RUNNING: AtomicBool = AtomicBool::new(false);

pub fn clean_kv() {
    let now = get_timestamp();
    KV.retain(|_, v| {
        if let Some(expiration) = v.expiration {
            if expiration < now {
                return false;
            }
        }
        true
    });
}

pub fn spawn_kv_cleaner() {
    // Check if cleaner is already running
    if CLEANER_RUNNING.load(std::sync::atomic::Ordering::SeqCst) {
        warn!("KV cleaner is already running");
        return;
    }

    CLEANER_RUNNING.store(true, std::sync::atomic::Ordering::SeqCst);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(120)).await;
            clean_kv();
        }
    });
}

#[instrument(level = "trace", skip_all)]
pub async fn read_from_store<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let key_param = eval_param("key", parameters, context)?;
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
    let key = eval_string_param("key", parameters, context)?;
    let value = eval_param("value", parameters, context)?;
    let expiration =
        eval_opt_u64_param("expiration", parameters, context)?.map(|v| v + get_timestamp());

    KV.insert(key, KvValue { value, expiration });

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn increment_store<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let key = eval_string_param("key", parameters, context)?;
    let increment = eval_opt_number_param("increment", parameters, context)?.unwrap_or(1.0);
    let expiration =
        eval_opt_u64_param("expiration", parameters, context)?.map(|v| v + get_timestamp());

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
    let key_param = eval_param("key", parameters, context)?;
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
                Parameter::tel("key", "\"expiration_key\""),
                Parameter::tel("value", "4"),
                Parameter::tel("expiration", "100"),
            ],
            "test",
        )
        .await
        .unwrap();

        read_from_store(
            &mut context,
            &vec![Parameter::tel("key", "\"expiration_key\"")],
            "test",
            Some("data"),
        )
        .await
        .unwrap();
        assert_eq!(
            context.storage.get("data"),
            Some(&StorageValue::Number(4.0))
        );

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        read_from_store(
            &mut context,
            &vec![Parameter::tel("key", "\"expiration_key\"")],
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
                Parameter::tel("key", "\"increment_key\""),
                Parameter::tel("increment", "2"),
            ],
            "test",
        )
        .await
        .unwrap();

        read_from_store(
            &mut context,
            &vec![Parameter::tel("key", "\"increment_key\"")],
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
            &vec![Parameter::tel("key", "\"increment_key\"")],
            "test",
        )
        .await
        .unwrap();

        read_from_store(
            &mut context,
            &vec![Parameter::tel("key", "\"increment_key\"")],
            "test",
            Some("data"),
        )
        .await
        .unwrap();
        assert_eq!(
            context.storage.get("data"),
            Some(&StorageValue::Number(3.0))
        );
    }
}

use crate::{
    errors::ExecutionError,
    evaluations::eval_u64_param,
    executor::{ExecutionContext, Parameter},
};
use chrono::{Datelike, Timelike, Utc};
use tel::{ObjectBody, StorageValue};
use tracing::instrument;

use super::store_value;

#[instrument(level = "trace", skip_all)]
pub async fn sleep<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
) -> Result<(), ExecutionError> {
    let time = eval_u64_param("milliseconds", parameters, context)?;

    tokio::time::sleep(tokio::time::Duration::from_millis(time)).await;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn get_current_time<'a>(
    context: &mut ExecutionContext<'a>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let value: StorageValue = (Utc::now().timestamp_millis() as f64).into();
    store_value(store_as, context, step_id, value).await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn get_current_datetime<'a>(
    context: &mut ExecutionContext<'a>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let now = Utc::now();
    let mut value: ObjectBody = ObjectBody::new();
    value.insert("year".to_owned(), now.year().into());
    value.insert("month".to_owned(), now.month().into());
    value.insert("day".to_owned(), now.day().into());
    value.insert("hour".to_owned(), now.hour().into());
    value.insert("minute".to_owned(), now.minute().into());
    value.insert("second".to_owned(), now.second().into());
    value.insert(
        "weekday".to_owned(),
        now.weekday().num_days_from_monday().into(),
    );

    store_value(store_as, context, step_id, StorageValue::Object(value)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{evaluations::eval, executor::ExecutionTest};

    use super::*;

    #[tokio::test]
    async fn test_sleep() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        let start = Utc::now().timestamp_millis();
        sleep(&mut context, &vec![Parameter::tel("milliseconds", "100")])
            .await
            .unwrap();
        let end = Utc::now().timestamp_millis();

        assert!(end - start >= 100);
    }

    #[tokio::test]
    async fn test_get_current_time() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        get_current_time(&mut context, "test", Some("time"))
            .await
            .unwrap();

        let time = eval("time", &context.storage, &context.environment).unwrap();

        assert!(matches!(time, crate::StorageValue::Number(_)));
    }

    #[tokio::test]
    async fn test_get_current_datetime() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        get_current_datetime(&mut context, "test", Some("time"))
            .await
            .unwrap();

        let time = eval("time", &context.storage, &context.environment).unwrap();

        assert!(matches!(time, crate::StorageValue::Object(_)));
    }
}

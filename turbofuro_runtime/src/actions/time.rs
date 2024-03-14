use crate::{
    errors::ExecutionError,
    evaluations::{eval_param, eval_saver_param},
    executor::{ExecutionContext, Parameter},
};
use chrono::Utc;
use tracing::instrument;

use super::as_integer;

#[instrument(level = "trace", skip_all)]
pub async fn sleep<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let time_param = eval_param(
        "milliseconds",
        parameters,
        &context.storage,
        &context.environment,
    )?;

    let time = as_integer(time_param, "milliseconds")?;

    if time < 0 {
        return Err(ExecutionError::ParameterInvalid {
            name: "milliseconds".to_owned(),
            message: "Milliseconds must be a positive integer".to_owned(),
        });
    }

    let millis: u64 = time
        .try_into()
        .map_err(|e| ExecutionError::ParameterInvalid {
            name: "Milliseconds".to_owned(),
            message: format!("Could not convert to milliseconds: {}", e),
        })?;

    tokio::time::sleep(tokio::time::Duration::from_millis(millis)).await;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn get_current_time<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
) -> Result<(), ExecutionError> {
    let selector = eval_saver_param(
        "saveAs",
        parameters,
        &mut context.storage,
        &context.environment,
    )?;

    context.add_to_storage(
        step_id,
        selector,
        (Utc::now().timestamp_millis() as f32).into(),
    )?;

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
        sleep(
            &mut context,
            &vec![Parameter::tel("milliseconds", "100")],
            "test",
        )
        .await
        .unwrap();
        let end = Utc::now().timestamp_millis();

        assert!(end - start >= 100);
    }

    #[tokio::test]
    async fn test_get_current_time() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        get_current_time(
            &mut context,
            &vec![Parameter::tel("saveAs", "time")],
            "test",
        )
        .await
        .unwrap();

        let time = eval("time", &context.storage, &context.environment).unwrap();

        assert!(matches!(time, crate::StorageValue::Number(_)));
    }
}

use tel::{describe, Description, StorageValue};
use tracing::instrument;

use crate::{
    errors::ExecutionError,
    evaluations::{eval_optional_param_with_default, eval_param, eval_saver_param},
    executor::{ExecutionContext, Parameter},
};

use super::as_string;

#[instrument(level = "trace", skip_all)]
pub async fn run_command(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
) -> Result<(), ExecutionError> {
    let program = eval_param(
        "program",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    let program = as_string(program, "program")?;

    let args = eval_optional_param_with_default(
        "args",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::Array(vec![]),
    )?;
    let args = match args {
        StorageValue::Array(a) => {
            let mut args = Vec::new();
            for arg in a {
                args.push(as_string(arg, "args")?);
            }
            Ok(args)
        }
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: "args".to_owned(),
            expected: Description::new_base_type("array"),
            actual: describe(s),
        }),
    }?;

    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    let output = cmd.output().await.map_err(ExecutionError::from)?;

    let output = StorageValue::Object(
        vec![
            (
                "stdout".to_owned(),
                StorageValue::String(String::from_utf8_lossy(&output.stdout).to_string()),
            ),
            (
                "stderr".to_owned(),
                StorageValue::String(String::from_utf8_lossy(&output.stderr).to_string()),
            ),
            (
                "status".to_owned(),
                StorageValue::Number(output.status.code().unwrap_or(-1).into()),
            ),
        ]
        .into_iter()
        .collect(),
    );

    let selector = eval_saver_param(
        "saveAs",
        parameters,
        &mut context.storage,
        &context.environment,
    )?;

    context.add_to_storage(step_id, selector, output)?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub fn set_environment_variable(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let key = eval_param("key", parameters, &context.storage, &context.environment)?;
    let key = as_string(key, "key")?;

    let value = eval_param("value", parameters, &context.storage, &context.environment)?;
    let value = as_string(value, "value")?;

    std::env::set_var(key, value);

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub fn read_environment_variable(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
) -> Result<(), ExecutionError> {
    let key = eval_param("key", parameters, &context.storage, &context.environment)?;
    let key = as_string(key, "key")?;

    let value: StorageValue = match std::env::var(key) {
        Ok(v) => Ok(v.into()),
        Err(e) => match e {
            std::env::VarError::NotPresent => Ok(StorageValue::Null(None)),
            std::env::VarError::NotUnicode(_) => Err(ExecutionError::StateInvalid {
                message: "Environment variable is not unicode".into(),
                subject: "environment".into(),
                inner: "NOT_UNICODE".into(),
            }),
        },
    }?;

    let selector = eval_saver_param(
        "saveAs",
        parameters,
        &mut context.storage,
        &context.environment,
    )?;

    context.add_to_storage(step_id, selector, value)?;

    Ok(())
}

#[cfg(test)]
mod test_os {
    use crate::{evaluations::eval, executor::ExecutionTest};

    use super::*;

    #[tokio::test]
    async fn test_run_command() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        run_command(
            &mut context,
            &vec![
                Parameter::tel("program", "\"echo\""),
                Parameter::tel("args", "[\"Hello World\"]"),
                Parameter::tel("saveAs", "output"),
            ],
            "test",
        )
        .await
        .unwrap();

        assert_eq!(
            eval("output.stdout", &context.storage, &context.environment).unwrap(),
            StorageValue::String("Hello World\n".to_owned())
        );
        assert_eq!(
            eval("output.status", &context.storage, &context.environment).unwrap(),
            StorageValue::Number(0.into())
        );
    }

    #[tokio::test]
    async fn test_read_environment_variable() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        std::env::set_var("TEST_VAR", "Test Value");

        read_environment_variable(
            &mut context,
            &vec![
                Parameter::tel("key", "\"TEST_VAR\""),
                Parameter::tel("saveAs", "value"),
            ],
            "test",
        )
        .unwrap();

        assert_eq!(
            eval("value", &context.storage, &context.environment).unwrap(),
            StorageValue::String("Test Value".to_owned())
        );
    }

    #[tokio::test]
    async fn test_set_environment_variable() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        set_environment_variable(
            &mut context,
            &vec![
                Parameter::tel("key", "\"TEST_VAR\""),
                Parameter::tel("value", "\"Test Value\""),
            ],
            "test",
        )
        .unwrap();

        assert_eq!(std::env::var("TEST_VAR").unwrap(), "Test Value");
    }
}

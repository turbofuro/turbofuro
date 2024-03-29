use tel::StorageValue;
use tokio::fs::OpenOptions;
use tracing::instrument;

use crate::{
    actions::as_string,
    errors::ExecutionError,
    evaluations::{eval_optional_param_with_default, eval_param},
    executor::{ExecutionContext, Parameter},
    resources::FileHandle,
};

use super::store_value;

#[instrument(level = "trace", skip_all)]
pub async fn open_file<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    // Get path
    let path_param = eval_param("path", parameters, &context.storage, &context.environment)?;
    let path = as_string(path_param, "path")?;

    // Get mode
    let mode_param = eval_optional_param_with_default(
        "mode",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::String("r".to_owned()),
    )?;
    let mode = as_string(mode_param, "mode")?;

    let mut open_options = OpenOptions::new();
    match mode.as_str() {
        "r" => open_options.read(true),
        "a" => open_options.append(true),
        "w" => open_options.write(true).create(true),
        "x" => open_options.write(true).create(false),
        _ => {
            return Err(ExecutionError::ParameterInvalid {
                name: "mode".to_owned(),
                message: format!(
                    "Unknown mode: {} allowed values are \"r\", \"a\", \"w\", \"x\"",
                    mode
                ),
            });
        }
    };

    let file = open_options
        .open(path)
        .await
        .map_err(ExecutionError::from)?;

    context.resources.files.push(FileHandle(file));

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn read_to_string<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    // Get path
    let path_param = eval_param("path", parameters, &context.storage, &context.environment)?;
    let path = as_string(path_param, "path")?;

    let data = tokio::fs::read_to_string(path.clone())
        .await
        .map_err(ExecutionError::from)?;

    store_value(store_as, context, step_id, data.into()).await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn write_string<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    // Get path
    let path_param = eval_param("path", parameters, &context.storage, &context.environment)?;
    let path = as_string(path_param, "path")?;

    // Get content
    let content = eval_param(
        "content",
        parameters,
        &context.storage,
        &context.environment,
    )?
    .to_string()
    .map_err(ExecutionError::from)?;

    tokio::fs::write(path, content)
        .await
        .map_err(ExecutionError::from)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{evaluations::eval, executor::ExecutionTest};

    use super::*;

    #[tokio::test]
    async fn test_write_and_read_string() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        write_string(
            &mut context,
            &vec![
                Parameter::tel("path", "\"test.txt\""),
                Parameter::tel("content", "\"This is a test message\""),
            ],
            "test",
        )
        .await
        .unwrap();

        read_to_string(
            &mut context,
            &vec![Parameter::tel("path", "\"test.txt\"")],
            "test",
            Some("data"),
        )
        .await
        .unwrap();

        assert_eq!(
            eval("data", &context.storage, &context.environment).unwrap(),
            "This is a test message".into()
        );

        let _ = tokio::fs::remove_file("test.txt").await;
    }
}

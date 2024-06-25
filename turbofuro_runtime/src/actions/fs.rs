use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use tel::{ObjectBody, StorageValue};
use tokio::{fs::OpenOptions, io::AsyncWriteExt};
use tracing::instrument;

use crate::{
    actions::as_string,
    errors::ExecutionError,
    evaluations::{eval_optional_param_with_default, eval_param},
    executor::{ExecutionContext, Parameter},
    resources::{FileHandle, Resource},
};

use super::store_value;

fn file_handle_not_found() -> ExecutionError {
    ExecutionError::MissingResource {
        resource_type: FileHandle::get_type().into(),
    }
}

fn system_time_to_millis_since_epoch(time: SystemTime) -> f64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as f64,
        Err(error) => -(error.duration().as_millis() as f64),
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn open_file<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
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

    let metadata = file.metadata().await.map_err(ExecutionError::from)?;

    let mut metadata_object: ObjectBody = ObjectBody::new();
    metadata_object.insert("size".into(), (metadata.len() as f64).into());
    metadata_object.insert(
        "created".into(),
        metadata
            .created()
            .ok()
            .map(|c| system_time_to_millis_since_epoch(c).into())
            .unwrap_or_default(),
    );
    metadata_object.insert(
        "modified".into(),
        metadata
            .modified()
            .ok()
            .map(|c| system_time_to_millis_since_epoch(c).into())
            .unwrap_or_default(),
    );

    context.resources.files.push(FileHandle { file });

    store_value(
        store_as,
        context,
        step_id,
        StorageValue::Object(metadata_object),
    )
    .await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn write_stream<'a>(
    context: &mut ExecutionContext<'a>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let mut file_handle = context
        .resources
        .files
        .pop()
        .ok_or(file_handle_not_found())?;

    let mut stream = context.resources.get_nearest_stream()?;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file_handle.file.write_all(&chunk).await?;
    }

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn simple_read_to_string<'a>(
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
pub async fn simple_write_string<'a>(
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

        simple_write_string(
            &mut context,
            &vec![
                Parameter::tel("path", "\"test.txt\""),
                Parameter::tel("content", "\"This is a test message\""),
            ],
            "test",
        )
        .await
        .unwrap();

        simple_read_to_string(
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

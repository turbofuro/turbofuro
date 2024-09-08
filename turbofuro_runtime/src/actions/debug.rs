use std::time::Duration;

use super::{as_string, store_value};
use crate::{
    errors::ExecutionError,
    evaluations::{eval_optional_param_with_default, eval_param},
    executor::{ExecutionContext, Parameter},
};
use nanoid::nanoid;
use tokio::{sync::oneshot, time::timeout};
use tracing::instrument;

#[instrument(level = "trace", skip_all)]
pub async fn ask_for_input(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let text = as_string(
        eval_param("text", parameters, &context.storage, &context.environment)?,
        "text",
    )?;
    let label = as_string(
        eval_param("label", parameters, &context.storage, &context.environment)?,
        "label",
    )?;
    let placeholder = as_string(
        eval_optional_param_with_default(
            "placeholder",
            parameters,
            &context.storage,
            &context.environment,
            "".into(),
        )?,
        "placeholder",
    )?;

    match &context.mode {
        crate::executor::ExecutionMode::Debug(handle) => {
            let (sender, receiver) = oneshot::channel();
            let _ = handle
                .sender
                .send(crate::debug::DebugMessage::AskForInput {
                    id: nanoid!(),
                    text,
                    label,
                    placeholder,
                    sender,
                })
                .await;

            match timeout(Duration::from_secs(300), receiver).await {
                Ok(result) => match result {
                    Ok(value) => {
                        store_value(store_as, context, _step_id, value).await?;
                    }
                    Err(_) => {
                        return Err(ExecutionError::Unknown {
                            message: "Timeout while waiting for input".to_owned(), // TODO: Better error
                        });
                    }
                },
                Err(_) => {
                    return Err(ExecutionError::Unknown {
                        message: "Timeout while waiting for input".to_owned(), // TODO: Better error
                    });
                }
            }
        }
        _ => {
            // No-op
        }
    }
    return Ok(());
}

#[instrument(level = "trace", skip_all)]
pub async fn show_result(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let value = eval_param("value", parameters, &context.storage, &context.environment)?;

    match &context.mode {
        crate::executor::ExecutionMode::Debug(handle) => {
            let _ = handle
                .sender
                .send(crate::debug::DebugMessage::ShowResult {
                    id: nanoid!(),
                    value,
                })
                .await;
        }
        _ => {
            // No-op
        }
    }
    return Ok(());
}

#[instrument(level = "trace", skip_all)]
pub async fn show_notification(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let text = as_string(
        eval_param("text", parameters, &context.storage, &context.environment)?,
        "text",
    )?;
    let variant = as_string(
        eval_param(
            "variant",
            parameters,
            &context.storage,
            &context.environment,
        )?,
        "variant",
    )?;

    match &context.mode {
        crate::executor::ExecutionMode::Debug(handle) => {
            let _ = handle
                .sender
                .send(crate::debug::DebugMessage::ShowNotification {
                    id: nanoid!(),
                    text,
                    variant,
                })
                .await;
        }
        _ => {
            // No-op
        }
    }
    return Ok(());
}

#[instrument(level = "trace", skip_all)]
pub async fn play_sound(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let sound = as_string(
        eval_param("sound", parameters, &context.storage, &context.environment)?,
        "sound",
    )?;

    match &context.mode {
        crate::executor::ExecutionMode::Debug(handle) => {
            let _ = handle
                .sender
                .send(crate::debug::DebugMessage::PlaySound {
                    id: nanoid!(),
                    sound,
                })
                .await;
        }
        _ => {
            // No-op
        }
    }
    return Ok(());
}

#[cfg(test)]
mod test_debug {
    use crate::executor::ExecutionTest;

    use super::*;

    #[tokio::test]
    async fn test_noop_when_debugger_is_not_active() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        play_sound(
            &mut context,
            &vec![Parameter::tel("sound", "alert")],
            "test",
            None,
        )
        .await
        .unwrap();

        show_notification(
            &mut context,
            &vec![
                Parameter::tel("text", "\"Hello World\""),
                Parameter::tel("variant", "\"success\""),
            ],
            "test",
            None,
        )
        .await
        .unwrap();

        show_result(
            &mut context,
            &vec![
                Parameter::tel("text", "\"Hello World\""),
                Parameter::tel("variant", "\"success\""),
            ],
            "test",
            None,
        )
        .await
        .unwrap();

        ask_for_input(
            &mut context,
            &vec![
                Parameter::tel("text", "Please enter your password"),
                Parameter::tel("label", "Password"),
            ],
            "test",
            None,
        )
        .await
        .unwrap();
    }
}

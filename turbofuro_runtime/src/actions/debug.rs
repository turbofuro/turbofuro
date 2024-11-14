use std::time::Duration;

use super::store_value;
use crate::{
    debug::DebugOption,
    errors::ExecutionError,
    evaluations::{eval_opt_string_param, eval_optional_param, eval_param, eval_string_param},
    executor::{ExecutionContext, Parameter},
};
use nanoid::nanoid;
use tel::{describe, Description, StorageValue};
use tokio::{sync::oneshot, time::timeout};
use tracing::instrument;

#[instrument(level = "trace", skip_all)]
pub async fn ask_for_value(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    match &context.mode {
        crate::executor::ExecutionMode::Debug(handle) => {
            let text = eval_string_param("text", parameters, context)?;
            let value =
                eval_optional_param("value", parameters, &context.storage, &context.environment)?;
            let title = eval_opt_string_param("title", parameters, context)?;
            let placeholder = eval_opt_string_param("placeholder", parameters, context)?;
            let label = eval_opt_string_param("label", parameters, context)?;
            let variant: Option<String> = eval_opt_string_param("variant", parameters, context)?;

            let (sender, receiver) = oneshot::channel();
            let _ = handle
                .sender
                .send(crate::debug::DebugMessage::AskForValue {
                    id: nanoid!(),
                    text,
                    label,
                    placeholder,
                    sender,
                    title,
                    value,
                    mode: "raw".to_owned(),
                    options: None,
                    variant,
                })
                .await;

            let value = timeout(Duration::from_secs(300), receiver)
                .await
                .map_err(|_| ExecutionError::DebugError {
                    message: "Timeout while waiting for value".to_owned(),
                })?
                .map_err(|_| ExecutionError::DebugError {
                    message: "Debug session ended before value was received".to_owned(),
                })?;

            store_value(store_as, context, step_id, value).await?;
        }
        _ => {
            return Err(ExecutionError::DebugError {
                message: "No debug active".to_owned(),
            });
        }
    }
    return Ok(());
}

#[instrument(level = "trace", skip_all)]
pub async fn ask_for_input(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    match &context.mode {
        crate::executor::ExecutionMode::Debug(handle) => {
            let text = eval_string_param("text", parameters, context)?;
            let value =
                eval_optional_param("value", parameters, &context.storage, &context.environment)?;
            let title = eval_opt_string_param("title", parameters, context)?;
            let placeholder = eval_opt_string_param("placeholder", parameters, context)?;
            let label = eval_opt_string_param("label", parameters, context)?;
            let mode = eval_opt_string_param("mode", parameters, context)?
                .unwrap_or_else(|| "text".to_owned());
            let variant: Option<String> = eval_opt_string_param("variant", parameters, context)?;

            let (sender, receiver) = oneshot::channel();
            let _ = handle
                .sender
                .send(crate::debug::DebugMessage::AskForValue {
                    id: nanoid!(),
                    text,
                    label,
                    placeholder,
                    sender,
                    mode,
                    title,
                    options: None,
                    value,
                    variant,
                })
                .await;

            let value = timeout(Duration::from_secs(300), receiver)
                .await
                .map_err(|_| ExecutionError::DebugError {
                    message: "Timeout while waiting for value".to_owned(),
                })?
                .map_err(|_| ExecutionError::DebugError {
                    message: "Debug session ended before value was received".to_owned(),
                })?;

            store_value(store_as, context, step_id, value).await?;
        }
        _ => {
            return Err(ExecutionError::DebugError {
                message: "No debug active".to_owned(),
            });
        }
    }
    return Ok(());
}

#[instrument(level = "trace", skip_all)]
pub async fn ask_for_confirmation(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    match &context.mode {
        crate::executor::ExecutionMode::Debug(handle) => {
            let text = eval_string_param("text", parameters, context)?;
            let title = eval_opt_string_param("title", parameters, context)?;
            let mode = eval_opt_string_param("mode", parameters, context)?
                .unwrap_or_else(|| "confirm".to_owned());
            let variant: Option<String> = eval_opt_string_param("variant", parameters, context)?;

            let (sender, receiver) = oneshot::channel();
            let _ = handle
                .sender
                .send(crate::debug::DebugMessage::AskForValue {
                    id: nanoid!(),
                    text,
                    sender,
                    mode,
                    title,
                    label: None,
                    placeholder: None,
                    options: None,
                    value: None,
                    variant,
                })
                .await;

            let value = timeout(Duration::from_secs(300), receiver)
                .await
                .map_err(|_| ExecutionError::DebugError {
                    message: "Timeout while waiting for value".to_owned(),
                })?
                .map_err(|_| ExecutionError::DebugError {
                    message: "Debug session ended before value was received".to_owned(),
                })?;

            store_value(store_as, context, step_id, value).await?;
        }
        _ => {
            return Err(ExecutionError::DebugError {
                message: "No debug active".to_owned(),
            });
        }
    }
    return Ok(());
}

pub fn eval_debug_options_param(
    name: &str,
    parameters: &Vec<Parameter>,
    context: &ExecutionContext<'_>,
) -> Result<Vec<DebugOption>, ExecutionError> {
    let value = eval_param(name, parameters, &context.storage, &context.environment)?;
    match value {
        StorageValue::Array(arr) => {
            let mut result = Vec::with_capacity(arr.len());
            for (i, item) in arr.iter().enumerate() {
                let this_option = format!("options[{}]", i);

                match item {
                    StorageValue::Object(option_object) => {
                        let label = option_object
                            .get("label")
                            .ok_or_else(|| ExecutionError::ParameterInvalid {
                                name: this_option.clone(),
                                message: "Missing label".to_owned(),
                            })?
                            .to_string()?;
                        let option_value = option_object.get("value").ok_or_else(|| {
                            ExecutionError::ParameterInvalid {
                                name: this_option.clone(),
                                message: "Missing value".to_owned(),
                            }
                        })?;
                        let description = option_object
                            .get("description")
                            .map(|sv| sv.to_string())
                            .transpose()?;
                        let icon = option_object
                            .get("icon")
                            .map(|sv| sv.to_string())
                            .transpose()?;
                        let theme = option_object
                            .get("theme")
                            .map(|sv| sv.to_string())
                            .transpose()?;

                        result.push(DebugOption {
                            label,
                            description,
                            icon,
                            theme,
                            value: option_value.clone(),
                        });
                    }
                    _ => {
                        return Err(ExecutionError::ParameterInvalid {
                            name: format!("options[{}]", i),
                            message: "Could not parse option that is not an object".to_owned(),
                        })
                    }
                }
            }
            Ok(result)
        }
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: "options".to_owned(),
            expected: Description::new_base_type("array"),
            actual: describe(s),
        }),
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn ask_to_choose(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    match &context.mode {
        crate::executor::ExecutionMode::Debug(handle) => {
            let text = eval_string_param("text", parameters, context)?;
            let title = eval_opt_string_param("title", parameters, context)?;
            let mode = eval_opt_string_param("mode", parameters, context)? // Mode can be radio/select/multiple
                .unwrap_or_else(|| "radio".to_owned());
            let value =
                eval_optional_param("value", parameters, &context.storage, &context.environment)?;
            let options: Vec<DebugOption> =
                eval_debug_options_param("options", parameters, context)?;
            let variant: Option<String> = eval_opt_string_param("variant", parameters, context)?;

            let (sender, receiver) = oneshot::channel();
            let _ = handle
                .sender
                .send(crate::debug::DebugMessage::AskForValue {
                    id: nanoid!(),
                    text,
                    sender,
                    title,
                    label: None,
                    placeholder: None,
                    options: Some(options),
                    value,
                    variant,
                    mode,
                })
                .await;

            let value = timeout(Duration::from_secs(300), receiver)
                .await
                .map_err(|_| ExecutionError::DebugError {
                    message: "Timeout while waiting for value".to_owned(),
                })?
                .map_err(|_| ExecutionError::DebugError {
                    message: "Debug session ended before value was received".to_owned(),
                })?;

            store_value(store_as, context, step_id, value).await?;
        }
        _ => {
            return Err(ExecutionError::DebugError {
                message: "No debug active".to_owned(),
            });
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
    match &context.mode {
        crate::executor::ExecutionMode::Debug(handle) => {
            let text = eval_opt_string_param("text", parameters, context)?;
            let value =
                eval_optional_param("value", parameters, &context.storage, &context.environment)?;
            let title = eval_opt_string_param("title", parameters, context)?;
            let variant: Option<String> = eval_opt_string_param("variant", parameters, context)?;

            let (sender, receiver) = oneshot::channel();
            let _ = handle
                .sender
                .send(crate::debug::DebugMessage::ShowResult {
                    id: nanoid!(),
                    text,
                    title,
                    value,
                    variant,
                    sender,
                })
                .await;

            timeout(Duration::from_secs(300), receiver)
                .await
                .map_err(|_| ExecutionError::DebugError {
                    message: "Timeout while waiting for result confirmation".to_owned(),
                })?
                .map_err(|_| ExecutionError::DebugError {
                    message: "Debug session ended before result confirmation was received"
                        .to_owned(),
                })?;
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
    match &context.mode {
        crate::executor::ExecutionMode::Debug(handle) => {
            let text = eval_string_param("text", parameters, context)?;
            let variant: Option<String> = eval_opt_string_param("variant", parameters, context)?;

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
    match &context.mode {
        crate::executor::ExecutionMode::Debug(handle) => {
            let sound = eval_string_param("sound", parameters, context)?;

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
    use tel::StorageValue;

    use crate::{
        debug::DebugMessage,
        executor::{DebuggerHandle, ExecutionTest},
    };

    use super::*;

    #[tokio::test]
    async fn test_noop_when_debugger_is_not_active() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        // Let's make sure that even without parameters the steps are no-ops
        play_sound(&mut context, &vec![], "test", None)
            .await
            .unwrap();

        show_notification(&mut context, &vec![], "test", None)
            .await
            .unwrap();

        show_result(&mut context, &vec![], "test", None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_throws_when_not_debugging() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        let err = ask_for_value(&mut context, &vec![], "test", None)
            .await
            .unwrap_err();

        assert_eq!(
            err,
            ExecutionError::DebugError {
                message: "No debug active".to_owned()
            }
        );

        let err = ask_for_confirmation(&mut context, &vec![], "test", None)
            .await
            .unwrap_err();

        assert_eq!(
            err,
            ExecutionError::DebugError {
                message: "No debug active".to_owned()
            }
        );

        assert_eq!(
            err,
            ExecutionError::DebugError {
                message: "No debug active".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn test_play_sound() {
        let mut t = ExecutionTest::default();
        let (debugger_handle, mut receiver) = DebuggerHandle::new();
        let mut context = t.get_debug_context(debugger_handle);

        play_sound(
            &mut context,
            &vec![Parameter::tel("sound", "\"alert\"")],
            "test",
            None,
        )
        .await
        .unwrap();

        let result = receiver.recv().await.unwrap();
        assert!(matches!(result, DebugMessage::PlaySound { sound, .. } if sound == "alert"));
    }

    #[tokio::test]
    async fn test_show_notification() {
        let mut t = ExecutionTest::default();
        let (debugger_handle, mut receiver) = DebuggerHandle::new();
        let mut context = t.get_debug_context(debugger_handle);

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

        let result = receiver.recv().await.unwrap();
        assert!(
            matches!(result, DebugMessage::ShowNotification { text, variant, .. } if text == "Hello World" && variant == Some("success".to_owned()))
        );
    }

    #[tokio::test]
    async fn test_show_result() {
        let mut t = ExecutionTest::default();
        let (debugger_handle, mut receiver) = DebuggerHandle::new();
        let mut context = t.get_debug_context(debugger_handle);

        tokio::spawn(async move {
            let result = receiver.recv().await.unwrap();
            match result {
                DebugMessage::ShowResult {
                    sender,
                    text,
                    title,
                    value,
                    variant,
                    ..
                } => {
                    assert!(value == Some("Hello World".into()));
                    assert!(text == Some("Query returned following result:".to_owned()));
                    assert!(title == Some("Query finished successfully".to_owned()));
                    assert!(variant == Some("success".to_owned()));
                    let _ = sender.send(StorageValue::String("test".to_owned()));
                }
                _ => panic!("Expected correct AskForValue message"),
            }
        });

        show_result(
            &mut context,
            &vec![
                Parameter::tel("value", "\"Hello World\""),
                Parameter::tel("text", "\"Query returned following result:\""),
                Parameter::tel("title", "\"Query finished successfully\""),
                Parameter::tel("variant", "\"success\""),
            ],
            "test",
            None,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_ask_for_value() {
        let mut t = ExecutionTest::default();
        let (debugger_handle, mut receiver) = DebuggerHandle::new();
        let mut context = t.get_debug_context(debugger_handle);

        tokio::spawn(async move {
            let result = receiver.recv().await.unwrap();
            match result {
                DebugMessage::AskForValue {
                    sender,
                    text,
                    label,
                    placeholder,
                    mode,
                    title,
                    value,
                    ..
                } => {
                    assert!(text == "Please enter your password");
                    assert!(label == Some("Password".to_owned()));
                    assert!(placeholder == Some("*****".to_owned()));
                    assert!(mode == *"text");
                    assert!(title == Some("Sign in".to_owned()));
                    assert!(value.is_none());
                    let _ = sender.send(StorageValue::String("test".to_owned()));
                }
                _ => panic!("Expected correct AskForValue message"),
            }
        });

        ask_for_input(
            &mut context,
            &vec![
                Parameter::tel("text", "\"Please enter your password\""),
                Parameter::tel("label", "\"Password\""),
                Parameter::tel("placeholder", "\"*****\""),
                Parameter::tel("mode", "\"text\""),
                Parameter::tel("title", "\"Sign in\""),
            ],
            "test",
            None,
        )
        .await
        .unwrap();
    }
}

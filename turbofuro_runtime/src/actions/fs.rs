use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use futures_util::StreamExt;
use notify::RecursiveMode;
use notify_debouncer_full::new_debouncer;
use tel::{ObjectBody, StorageValue};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::mpsc};
use tracing::{debug, instrument, warn};

use crate::{
    actor::ActorCommand,
    errors::ExecutionError,
    evaluations::{
        eval_byte_array_param, eval_opt_boolean_param, eval_opt_string_param, eval_opt_u64_param,
        eval_param, eval_string_param, get_optional_handler_from_parameters,
    },
    executor::{ExecutionContext, Parameter},
    resources::{generate_resource_id, Cancellation, CancellationSubject, FileHandle, Resource},
};

use super::store_value;

fn system_time_to_millis_since_epoch(time: SystemTime) -> f64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as f64,
        Err(error) => -(error.duration().as_millis() as f64),
    }
}

static WATCHER_ID: AtomicU64 = AtomicU64::new(0);

pub fn cancellation_name(watcher_id: u64) -> String {
    format!("watcher_{}", watcher_id)
}

#[instrument(level = "trace", skip_all)]
pub async fn open_file<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let path = eval_string_param("path", parameters, context)?;
    let mode = eval_opt_string_param("mode", parameters, context)?.unwrap_or("r".to_owned());

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
    metadata_object.insert("isDir".into(), metadata.is_dir().into());
    metadata_object.insert("isFile".into(), metadata.is_file().into());

    context.resources.add_file(FileHandle {
        id: generate_resource_id(),
        file,
    });

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
pub async fn read_dir<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let path = eval_string_param("path", parameters, context)?;

    let mut dir = tokio::fs::read_dir(path)
        .await
        .map_err(ExecutionError::from)?;

    let mut entries = Vec::new();
    while let Some(entry) = dir.next_entry().await.map_err(ExecutionError::from)? {
        let metadata = entry.metadata().await.map_err(ExecutionError::from)?;
        let file_type = metadata.file_type();

        let mut entry_object = ObjectBody::new();
        entry_object.insert(
            "name".to_owned(),
            entry.file_name().to_string_lossy().to_string().into(),
        );
        entry_object.insert(
            "path".to_owned(),
            entry.path().as_path().to_string_lossy().to_string().into(),
        );
        entry_object.insert("isDir".to_owned(), file_type.is_dir().into());
        entry_object.insert("isFile".to_owned(), file_type.is_file().into());
        entry_object.insert("isSymlink".to_owned(), file_type.is_symlink().into());
        entries.push(StorageValue::Object(entry_object));
    }

    store_value(store_as, context, step_id, StorageValue::Array(entries)).await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn write_stream<'a>(
    context: &mut ExecutionContext<'a>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let mut file_handle = context
        .resources
        .pop_file()
        .ok_or_else(FileHandle::missing)?;

    let (mut stream, metadata) = context.resources.get_stream()?;

    context
        .note_resource_consumed(metadata.id, metadata.type_)
        .await;

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
    let path = eval_string_param("path", parameters, context)?;

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
    let path = eval_string_param("path", parameters, context)?;

    // Get content
    let content = eval_param("content", parameters, context)?
        .to_string()
        .map_err(ExecutionError::from)?;

    tokio::fs::write(path, content)
        .await
        .map_err(ExecutionError::from)?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn simple_read_to_bytes<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let path = eval_string_param("path", parameters, context)?;

    let data = tokio::fs::read(path.clone())
        .await
        .map_err(ExecutionError::from)?;

    store_value(
        store_as,
        context,
        step_id,
        StorageValue::new_byte_array(&data),
    )
    .await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn simple_write_bytes<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let path = eval_string_param("path", parameters, context)?;

    // Get content
    let content = eval_byte_array_param("content", parameters, context)?;

    tokio::fs::write(path, content)
        .await
        .map_err(ExecutionError::from)?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn setup_watcher<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let path = eval_string_param("path", parameters, context)?;
    let recursive = eval_opt_boolean_param("recursive", parameters, context)?.unwrap_or(false);
    let recursive_mode: RecursiveMode = match recursive {
        true => RecursiveMode::Recursive,
        false => RecursiveMode::NonRecursive,
    };
    let debounce = eval_opt_u64_param("debounceTime", parameters, context)?.unwrap_or(500);

    let function_ref = get_optional_handler_from_parameters("onMessage", parameters);
    let actor_id = context.actor_id.clone();
    let global = context.global.clone();

    let (tx, mut rx) = mpsc::channel(1);

    let watcher_id = WATCHER_ID.fetch_add(1, Ordering::AcqRel);

    let mut debouncer = new_debouncer(
        Duration::from_millis(debounce),
        None,
        move |results| match tx.blocking_send(results) {
            Ok(()) => {}
            Err(e) => warn!("Watcher could not send event: {:?}", e),
        },
    )?;

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    debouncer.watch(path, recursive_mode)?;

    tokio::spawn(async move {
        while let Some(result) = rx.recv().await {
            match result {
                Ok(debounced_events) => {
                    for debounced_event in debounced_events {
                        let event = debounced_event.event;

                        let messenger = {
                            global
                                .registry
                                .actors
                                .get(&actor_id)
                                .map(|r| r.value().clone())
                        };

                        // Send message
                        if let Some(messenger) = messenger {
                            let mut storage = ObjectBody::new();
                            let mut event_object = ObjectBody::new();

                            let paths: Vec<StorageValue> = event
                                .paths
                                .into_iter()
                                .map(|f| StorageValue::String(f.to_string_lossy().to_string()))
                                .collect();

                            if paths.len() == 1 {
                                event_object.insert("path".to_owned(), paths[0].clone());
                            } else {
                                event_object.insert("paths".to_owned(), StorageValue::Array(paths));
                            }

                            if let Some(tracker) = event.attrs.tracker() {
                                event_object.insert("tracker".to_owned(), tracker.into());
                            }

                            if let Some(source) = event.attrs.source() {
                                event_object.insert("source".to_owned(), source.into());
                            }

                            if let Some(info) = event.attrs.info() {
                                event_object.insert("info".to_owned(), info.into());
                            }

                            match event.kind {
                                notify::EventKind::Any => {
                                    event_object.insert("type".to_owned(), "any".into());
                                }
                                notify::EventKind::Access(kind) => {
                                    event_object.insert("type".to_owned(), "access".into());

                                    let kind = match kind {
                                        notify::event::AccessKind::Any => "any",
                                        notify::event::AccessKind::Read => "read",
                                        notify::event::AccessKind::Open(_) => "open",
                                        notify::event::AccessKind::Close(_) => "close",
                                        notify::event::AccessKind::Other => "other",
                                    };
                                    event_object.insert("kind".to_owned(), kind.into());
                                }
                                notify::EventKind::Create(kind) => {
                                    event_object.insert("type".to_owned(), "create".into());

                                    let kind = match kind {
                                        notify::event::CreateKind::Any => "any",
                                        notify::event::CreateKind::File => "file",
                                        notify::event::CreateKind::Folder => "folder",
                                        notify::event::CreateKind::Other => "other",
                                    };
                                    event_object.insert("kind".to_owned(), kind.into());
                                }
                                notify::EventKind::Modify(kind) => {
                                    event_object.insert("type".to_owned(), "modify".into());

                                    let kind = match kind {
                                        notify::event::ModifyKind::Any => "any",
                                        notify::event::ModifyKind::Data(_) => "data",
                                        notify::event::ModifyKind::Metadata(_) => "metadata",
                                        notify::event::ModifyKind::Name(_) => "name",
                                        notify::event::ModifyKind::Other => "other",
                                    };
                                    event_object.insert("kind".to_owned(), kind.into());
                                }
                                notify::EventKind::Remove(kind) => {
                                    event_object.insert("type".to_owned(), "remove".into());

                                    let kind = match kind {
                                        notify::event::RemoveKind::Any => "any",
                                        notify::event::RemoveKind::File => "file",
                                        notify::event::RemoveKind::Folder => "folder",
                                        notify::event::RemoveKind::Other => "other",
                                    };
                                    event_object.insert("kind".to_owned(), kind.into());
                                }
                                notify::EventKind::Other => {
                                    event_object.insert("type".to_owned(), "other".into());
                                }
                            };
                            storage.insert("event".to_owned(), StorageValue::Object(event_object));

                            if let Some(ref function_ref) = function_ref {
                                let result = messenger
                                    .send(ActorCommand::RunFunctionRef {
                                        function_ref: function_ref.clone(),
                                        storage,
                                        references: HashMap::new(),
                                        sender: None,
                                        execution_id: None,
                                    })
                                    .await;

                                if let Err(e) = result {
                                    warn!("Could not run file watcher handler: {}", e);
                                    break;
                                }
                            } else {
                                let result = messenger
                                    .send(ActorCommand::Run {
                                        handler: "onMessage".to_owned(),
                                        storage,
                                        references: HashMap::new(),
                                        sender: None,
                                        execution_id: None,
                                    })
                                    .await;

                                if let Err(e) = result {
                                    warn!("Could not run file watcher handler: {}", e);
                                    break;
                                }
                            }
                        } else {
                            debug!("Alarm fired but actor {} was not found", actor_id)
                        }
                    }
                }
                Err(errors) => {
                    // TODO: Handle errors
                    warn!("Watcher errors: {:?}", errors);
                }
            }
        }
        debug!("Watcher ended");
    });

    // Canceller
    let (sender, receiver) = tokio::sync::oneshot::channel::<()>();
    let cancellation_id = generate_resource_id();
    context.resources.add_cancellation(Cancellation {
        id: cancellation_id,
        sender,
        name: cancellation_name(watcher_id),
        subject: CancellationSubject::Watcher,
    });
    context
        .note_resource_provisioned(cancellation_id, Cancellation::static_type())
        .await;

    // Spawn task to wait for cancellation
    tokio::spawn(async move {
        match receiver.await {
            Ok(_) => {
                debug!("Cancellation received for file watcher {}", watcher_id);
            }
            Err(_) => {
                debug!(
                    "Cancellation sender dropped for file watcher {}",
                    watcher_id
                );
            }
        }
        drop(debouncer);
    });

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn cancel_watcher<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &[Parameter],
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let cancellation = context
        .resources
        .pop_cancellation_where(|c| matches!(c.subject, CancellationSubject::Watcher))
        .ok_or_else(Cancellation::missing)?;
    context
        .note_resource_consumed(cancellation.id, Cancellation::static_type())
        .await;

    cancellation
        .sender
        .send(())
        .map_err(|_| ExecutionError::StateInvalid {
            message: "Failed to send cancel signal to watcher".to_owned(),
            subject: Cancellation::static_type().into(),
            inner: "Send error".to_owned(),
        })?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn create_directory<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let path = eval_string_param("path", parameters, context)?;
    let recursive = eval_opt_boolean_param("recursive", parameters, context)?.unwrap_or(false);

    if recursive {
        tokio::fs::create_dir_all(path).await?;
    } else {
        tokio::fs::create_dir(path).await?;
    }

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn rename<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let from = eval_string_param("from", parameters, context)?;
    let to = eval_string_param("to", parameters, context)?;

    tokio::fs::rename(from, to).await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn remove_file<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let path = eval_string_param("path", parameters, context)?;

    tokio::fs::remove_file(path).await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn remove_directory<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let path = eval_string_param("path", parameters, context)?;
    let recursive = eval_opt_boolean_param("recursive", parameters, context)?.unwrap_or(false);

    if recursive {
        tokio::fs::remove_dir_all(path).await?;
    } else {
        tokio::fs::remove_dir(path).await?;
    }

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn copy<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let from = eval_string_param("from", parameters, context)?;
    let to = eval_string_param("to", parameters, context)?;

    tokio::fs::copy(from, to).await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn canonicalize<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let path = eval_string_param("path", parameters, context)?;

    let canonicalized = tokio::fs::canonicalize(path).await?;
    let canonicalized = canonicalized.to_string_lossy().to_string();

    store_value(store_as, context, step_id, canonicalized.into()).await?;

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

    #[tokio::test]
    async fn test_copy_rename_remove_file() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        simple_write_string(
            &mut context,
            &vec![
                Parameter::tel("path", "\"test_a.txt\""),
                Parameter::tel("content", "\"This is a test message\""),
            ],
            "test_write",
        )
        .await
        .unwrap();

        copy(
            &mut context,
            &vec![
                Parameter::tel("from", "\"test_a.txt\""),
                Parameter::tel("to", "\"test_b.txt\""),
            ],
            "test_copy",
        )
        .await
        .unwrap();

        remove_file(
            &mut context,
            &vec![Parameter::tel("path", "\"test_a.txt\"")],
            "test_remove_file_a",
        )
        .await
        .unwrap();

        rename(
            &mut context,
            &vec![
                Parameter::tel("from", "\"test_b.txt\""),
                Parameter::tel("to", "\"test_c.txt\""),
            ],
            "test_rename",
        )
        .await
        .unwrap();

        remove_file(
            &mut context,
            &vec![Parameter::tel("path", "\"test_c.txt\"")],
            "test_remove_file_c",
        )
        .await
        .unwrap();

        assert!(!tokio::fs::try_exists("test_a.txt").await.unwrap());
        assert!(!tokio::fs::try_exists("test_b.txt").await.unwrap());
        assert!(!tokio::fs::try_exists("test_c.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_create_remove_directory() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        create_directory(
            &mut context,
            &vec![
                Parameter::tel("path", "\"test_dir/test_subdir\""),
                Parameter::tel("recursive", "true"),
            ],
            "test_mkdir",
        )
        .await
        .unwrap();

        assert!(tokio::fs::try_exists("test_dir/test_subdir").await.unwrap());

        remove_directory(
            &mut context,
            &vec![
                Parameter::tel("path", "\"test_dir/test_subdir\""),
                Parameter::tel("recursive", "true"),
            ],
            "test_mkdir",
        )
        .await
        .unwrap();

        assert!(!tokio::fs::try_exists("test_dir/test_subdir").await.unwrap());
    }

    #[tokio::test]
    async fn test_canonicalize() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        canonicalize(
            &mut context,
            &vec![
                Parameter::tel("path", "\"src/actions/fs.rs\""),
                Parameter::tel("content", "\"This is a test message\""),
            ],
            "test_write",
            Some("path"),
        )
        .await
        .unwrap();

        let path = context.storage.get("path").unwrap().to_string().unwrap();
        assert!(path.ends_with("turbofuro_runtime/src/actions/fs.rs"));
    }
}

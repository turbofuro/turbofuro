use crate::{
    actor::ActorCommand,
    errors::ExecutionError,
    evaluations::{
        eval_integer_param, eval_optional_param_with_default, eval_string_param,
        get_optional_handler_from_parameters,
    },
    executor::{ExecutionContext, Global, Parameter},
    resources::{generate_resource_id, Cancellation, CancellationSubject, Resource},
};
use chrono::Local;
use croner::Cron;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tel::{ObjectBody, StorageValue};
use tracing::{debug, instrument, warn};

static ALARM_ID: AtomicU64 = AtomicU64::new(0);

pub fn cancellation_name(alarm_id: u64) -> String {
    format!("alarm_{}", alarm_id)
}

#[instrument(level = "trace", skip_all)]
pub async fn set_alarm<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let time = eval_integer_param("timeout", parameters, context)?;
    if time < 0 {
        return Err(ExecutionError::ParameterInvalid {
            name: "timeout".to_owned(),
            message: "Timeout must be a positive integer".to_owned(),
        });
    }
    let millis: u64 = time
        .try_into()
        .map_err(|e| ExecutionError::ParameterInvalid {
            name: "timeout".to_owned(),
            message: format!("Could not convert to milliseconds: {}", e),
        })?;

    let data =
        eval_optional_param_with_default("data", parameters, context, StorageValue::Null(None))?;
    let function_ref = get_optional_handler_from_parameters("onTimeout", parameters);

    let actor_id = context.actor_id.clone();
    let global = context.global.clone();

    let alarm_id = ALARM_ID.fetch_add(1, Ordering::AcqRel);

    // Cancellation
    let (sender, receiver) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_millis(millis)) => {
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
                    storage.insert("message".to_owned(), data);

                    if let Some(function_ref) = function_ref {
                        messenger
                            .send(ActorCommand::RunFunctionRef {
                                function_ref,
                                storage,
                                references: HashMap::new(),
                                sender: None,
                                execution_id: None,
                            })
                            .await
                            .unwrap();
                    } else {
                        messenger
                            .send(ActorCommand::Run {
                                handler: "onMessage".to_owned(),
                                storage,
                                references: HashMap::new(),
                                sender: None,
                                execution_id: None,
                            })
                            .await
                            .unwrap();
                    }
                } else {
                    debug!("Alarm fired but actor {} was not found", actor_id)
                }
            }
            _ = receiver => {
                // Noop
                debug!("Alarm cancelled")
            }
        }
    });

    context.resources.add_cancellation(Cancellation {
        id: generate_resource_id(),
        name: cancellation_name(alarm_id),
        sender,
        subject: CancellationSubject::Alarm,
    });

    Ok(())
}

async fn run_interval_inner(
    global: Arc<Global>,
    actor_id: String,
    data: StorageValue,
    interval: u64,
    function_ref: Option<String>,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(interval));
    interval.tick().await; // Skip first tick
    loop {
        interval.tick().await;

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
            storage.insert("message".to_owned(), data.clone());

            if let Some(function_ref) = function_ref.clone() {
                messenger
                    .send(ActorCommand::RunFunctionRef {
                        function_ref,
                        storage,
                        references: HashMap::new(),
                        sender: None,
                        execution_id: None,
                    })
                    .await
                    .unwrap();
            } else {
                messenger
                    .send(ActorCommand::Run {
                        handler: "onMessage".to_owned(),
                        storage,
                        references: HashMap::new(),
                        sender: None,
                        execution_id: None,
                    })
                    .await
                    .unwrap();
            }
        }
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn set_interval<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let time = eval_integer_param("interval", parameters, context)?;
    if time < 0 {
        return Err(ExecutionError::ParameterInvalid {
            name: "interval".to_owned(),
            message: "Interval must be a positive integer".to_owned(),
        });
    }
    let millis: u64 = time
        .try_into()
        .map_err(|e| ExecutionError::ParameterInvalid {
            name: "interval".to_owned(),
            message: format!("Could not convert to milliseconds: {}", e),
        })?;

    let data =
        eval_optional_param_with_default("data", parameters, context, StorageValue::Null(None))?;
    let function_ref = get_optional_handler_from_parameters("onTimeout", parameters);

    let actor_id = context.actor_id.clone();
    let global = context.global.clone();

    let alarm_id = ALARM_ID.fetch_add(1, Ordering::AcqRel);

    // Cancellation
    let (sender, receiver) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        tokio::select! {
            _ = run_interval_inner(
                global,
                actor_id,
                data,
                millis,
                function_ref,
            ) => {
                debug!("Interval finished");
            }
            _ = receiver => {
                // Noop
                debug!("Alarm cancelled");
            }
        }
    });

    context.resources.add_cancellation(Cancellation {
        id: generate_resource_id(),
        name: cancellation_name(alarm_id),
        sender,
        subject: CancellationSubject::Alarm,
    });

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn cancel_alarm<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &[Parameter],
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let cancellation = context
        .resources
        .pop_cancellation_where(|c| matches!(c.subject, CancellationSubject::Alarm))
        .ok_or_else(Cancellation::missing)?;

    cancellation
        .sender
        .send(())
        .map_err(|_| ExecutionError::StateInvalid {
            message: "Could not send cancel alarm".to_owned(),
            subject: "alarm".to_owned(),
            inner: "Send error".to_owned(),
        })?;

    Ok(())
}

async fn run_cronjob_inner(
    global: Arc<Global>,
    actor_id: String,
    data: StorageValue,
    function_ref: Option<String>,
    cron: Cron,
) {
    loop {
        let now = Local::now();
        match cron.find_next_occurrence(&now, false) {
            Ok(next) => {
                // Sleep until next schedule
                let duration = next - now;
                let duration = match duration.to_std() {
                    Ok(dur) => dur,
                    Err(e) => {
                        warn!(
                            "Cronjob on actor {:?} is in the past, skipping. Error was {:?}",
                            actor_id, e
                        );
                        continue;
                    }
                };

                tokio::time::sleep(duration).await;

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
                    storage.insert("message".to_owned(), data.clone());

                    if let Some(ref function_ref) = function_ref {
                        messenger
                            .send(ActorCommand::RunFunctionRef {
                                function_ref: function_ref.clone(),
                                storage,
                                references: HashMap::new(),
                                sender: None,
                                execution_id: None,
                            })
                            .await
                            .unwrap();
                    } else {
                        messenger
                            .send(ActorCommand::Run {
                                handler: "onMessage".to_owned(),
                                storage,
                                references: HashMap::new(),
                                sender: None,
                                execution_id: None,
                            })
                            .await
                            .unwrap();
                    }
                }
            }
            Err(e) => {
                warn!("Could not find next schedule: {}", e);
                break;
            }
        }
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn setup_cronjob<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let schedule = eval_string_param("schedule", parameters, context)?;
    let data =
        eval_optional_param_with_default("data", parameters, context, StorageValue::Null(None))?;
    let function_ref = get_optional_handler_from_parameters("onSchedule", parameters);

    let cron =
        Cron::new(schedule.as_str())
            .parse()
            .map_err(|e| ExecutionError::ParameterInvalid {
                name: "schedule".to_owned(),
                message: format!("Could not parse {} as a CRON expression", e),
            })?;

    let actor_id = context.actor_id.clone();
    let global = context.global.clone();

    // Before we start the cronjob, let's take a look if it makes sense to start it
    let now = Local::now();
    if let Err(err) = cron.find_next_occurrence(&now, false) {
        return Err(ExecutionError::ParameterInvalid {
            name: "schedule".to_owned(),
            message: format!("Could not find next schedule: {}", err),
        });
    }

    let alarm_id = ALARM_ID.fetch_add(1, Ordering::AcqRel);

    // Cancellation
    let (sender, receiver) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        tokio::select! {
            _ = run_cronjob_inner(
                global,
                actor_id,
                data,
                function_ref,
                cron
            ) => {
                debug!("Cronjob finished");
            }
            _ = receiver => {
                // Noop
                debug!("Cronjob cancelled");
            }
        }
    });

    context.resources.add_cancellation(Cancellation {
        id: generate_resource_id(),
        name: cancellation_name(alarm_id),
        sender,
        subject: CancellationSubject::Cronjob,
    });

    Ok(())
}

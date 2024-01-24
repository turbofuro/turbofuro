use crate::{
    actor::ActorCommand,
    errors::ExecutionError,
    evaluations::{eval_optional_param_with_default, eval_param},
    executor::{ExecutionContext, Global, Parameter},
    resources::{Cancellation, CancellationSubject},
};
use chrono::Utc;
use cron::Schedule;
use std::{
    str::FromStr,
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

use super::{as_integer, get_optional_handler_from_parameters};

#[instrument(level = "trace", skip_all)]
pub async fn set_alarm<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let time_param = eval_param(
        "timeout",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    let time = as_integer(time_param, "timeout")?;
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

    let data = eval_optional_param_with_default(
        "data",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::Null(None),
    )?;
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
                    .map(|r| r.value().0.clone())
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
                                sender: None,
                            })
                            .await
                            .unwrap();
                    } else {
                        messenger
                            .send(ActorCommand::Run {
                                handler: "onMessage".to_owned(),
                                storage,
                                sender: None,
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

    context.resources.cancellations.push(Cancellation {
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
                .map(|r| r.value().0.clone())
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
                        sender: None,
                    })
                    .await
                    .unwrap();
            } else {
                messenger
                    .send(ActorCommand::Run {
                        handler: "onMessage".to_owned(),
                        storage,
                        sender: None,
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
    let time_param = eval_param(
        "interval",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    let time = as_integer(time_param, "interval")?;
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

    let data = eval_optional_param_with_default(
        "data",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::Null(None),
    )?;
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

    context.resources.cancellations.push(Cancellation {
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
    let index = context
        .resources
        .cancellations
        .iter()
        .position(|c| matches!(c.subject, CancellationSubject::Alarm));

    if let Some(i) = index {
        let cancellation = context.resources.cancellations.remove(i);
        cancellation.sender.send(()).unwrap();
    } else {
        return Err(ExecutionError::MissingResource {
            resource_type: "cancellation".into(),
        });
    }

    Ok(())
}

async fn run_cronjob_inner(
    global: Arc<Global>,
    actor_id: String,
    data: StorageValue,
    function_ref: Option<String>,
    schedule: Schedule,
) {
    while let Some(schedule) = schedule.upcoming(Utc).next() {
        // Sleep until next schedule
        let now = Utc::now();
        let duration = schedule - now;

        if duration.num_milliseconds() < 0 {
            // Schedule is in the past (?), skip
            warn!("Cronjob with schedule {:?} is in the past", schedule);
            continue;
        }
        let duration = duration.to_std().unwrap();
        tokio::time::sleep(duration).await;

        let messenger = {
            global
                .registry
                .actors
                .get(&actor_id)
                .map(|r| r.value().0.clone())
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
                        sender: None,
                    })
                    .await
                    .unwrap();
            } else {
                messenger
                    .send(ActorCommand::Run {
                        handler: "onMessage".to_owned(),
                        storage,
                        sender: None,
                    })
                    .await
                    .unwrap();
            }
        }
    }
    warn!("Cronjob with schedule {:?} has no upcoming date", schedule);
}

#[instrument(level = "trace", skip_all)]
pub async fn setup_cronjob<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let schedule_param = eval_param(
        "schedule",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    let schedule = schedule_param.to_string()?;
    let data = eval_optional_param_with_default(
        "data",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::Null(None),
    )?;
    let function_ref = get_optional_handler_from_parameters("onSchedule", parameters);
    let schedule =
        Schedule::from_str(schedule.as_str()).map_err(|e| ExecutionError::ParameterInvalid {
            name: "schedule".to_owned(),
            message: format!("Could not parse {} as a CRON expression", e),
        })?;

    let actor_id = context.actor_id.clone();
    let global = context.global.clone();

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
                schedule
            ) => {
                debug!("Cronjob finished");
            }
            _ = receiver => {
                // Noop
                debug!("Cronjob cancelled");
            }
        }
    });

    context.resources.cancellations.push(Cancellation {
        name: cancellation_name(alarm_id),
        sender,
        subject: CancellationSubject::Cronjob,
    });

    Ok(())
}

use crate::{
    actor::ActorCommand,
    errors::ExecutionError,
    evaluations::{
        eval_opt_u64_param, eval_optional_param_with_default, get_handler_from_parameters,
    },
    executor::{ExecutionContext, Global, Parameter},
    resources::{Cancellation, CancellationSubject, Resource},
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tel::{ObjectBody, StorageValue};
use tracing::{debug, instrument};

static TASK_ID: AtomicU64 = AtomicU64::new(0);

pub fn cancellation_name(task_id: u64) -> String {
    format!("task_{}", task_id)
}

#[instrument(level = "trace", skip_all)]
pub async fn run_task_continuously<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let backoff = eval_opt_u64_param("backoff", parameters, context)?.unwrap_or(5000);

    let data =
        eval_optional_param_with_default("data", parameters, context, StorageValue::Null(None))?;
    let function_ref = get_handler_from_parameters("onRun", parameters)?;

    let actor_id = context.actor_id.clone();
    let global = context.global.clone();

    let task_id = TASK_ID.fetch_add(1, Ordering::AcqRel);

    // Cancellation
    let (sender, receiver) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        tokio::select! {
            _ = run_task_continuously_inner(
                global,
                actor_id,
                data,
                backoff,
                function_ref,
            ) => {
                debug!("Continuous task finished");
            }
            _ = receiver => {
                // Noop
                debug!("Task cancelled");
            }
        }
    });

    context.resources.cancellations.push(Cancellation {
        name: cancellation_name(task_id),
        sender,
        subject: CancellationSubject::Task,
    });

    Ok(())
}

async fn run_task_continuously_inner(
    global: Arc<Global>,
    actor_id: String,
    data: StorageValue,
    backoff: u64,
    function_ref: String,
) {
    loop {
        let messenger = {
            global
                .registry
                .actors
                .get(&actor_id)
                .map(|r| r.value().clone())
        };

        let (sender, receiver) = tokio::sync::oneshot::channel();

        // Send message
        if let Some(messenger) = messenger {
            let mut storage = ObjectBody::new();
            storage.insert("data".to_owned(), data.clone());

            messenger
                .send(ActorCommand::RunFunctionRef {
                    function_ref: function_ref.clone(),
                    storage,
                    references: HashMap::new(),
                    sender: Some(sender),
                    execution_id: None,
                })
                .await
                .unwrap();
        }

        match receiver.await.unwrap() {
            Ok(_) => {
                // Noop
            }
            Err(_) => {
                // Noop
                tokio::time::sleep(Duration::from_millis(backoff)).await;
            }
        }
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn cancel_task<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &[Parameter],
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let index = context
        .resources
        .cancellations
        .iter()
        .position(|c| matches!(c.subject, CancellationSubject::Task));

    if let Some(i) = index {
        let cancellation = context.resources.cancellations.remove(i);
        cancellation.sender.send(()).unwrap();
    } else {
        return Err(Cancellation::missing());
    }

    Ok(())
}

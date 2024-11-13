use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
};

use crate::{
    actor::ActorCommand,
    evaluations::{
        eval_opt_string_param, eval_optional_param_with_default, eval_string_param,
        get_optional_handler_from_parameters,
    },
    resources::{Cancellation, CancellationSubject},
};
use tel::{ObjectBody, StorageValue, NULL};
use tracing::{debug, error, instrument};

use crate::{
    errors::ExecutionError,
    evaluations::eval_param,
    executor::{ExecutionContext, Global, Parameter},
};

fn cancellation_name(channel: &str) -> String {
    format!("pubsub_{}", channel)
}

#[instrument(level = "trace", skip_all)]
pub async fn publish<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let channel = eval_string_param("channel", parameters, context)?;
    let value = eval_param("value", parameters, &context.storage, &context.environment)?;

    let pub_sub = context.global.pub_sub.lock().await;
    pub_sub
        .get(&channel)
        .map(|tx| tx.send(value).unwrap_or(0))
        .unwrap_or(0);

    Ok(())
}

async fn subscribe_and_schedule_runs(
    global: Arc<Global>,
    actor_id: String,
    function_ref: Option<String>,
    context: StorageValue,
    mut receiver: tokio::sync::broadcast::Receiver<StorageValue>,
) {
    loop {
        let value = receiver.recv().await;
        match value {
            Ok(value) => {
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
                    storage.insert("message".to_owned(), value);
                    storage.insert("context".to_owned(), context.clone());

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
                } else {
                    debug!("Actor {} not found", actor_id);
                    break;
                }
            }
            Err(e) => match e {
                tokio::sync::broadcast::error::RecvError::Closed => {
                    break;
                }
                tokio::sync::broadcast::error::RecvError::Lagged(e) => {
                    error!(
                        "Actor {} lagged for {} messages for its PubSub subscription",
                        actor_id, e
                    );
                    // TODO: How to communicate this to the actor? Another handle like onLag?
                }
            },
        }
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn subscribe<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let channel = eval_string_param("channel", parameters, context)?;
    let handler = get_optional_handler_from_parameters("onMessage", parameters);

    let context_param = eval_optional_param_with_default(
        "context",
        parameters,
        &context.storage,
        &context.environment,
        NULL,
    )?;

    let mut pub_sub = context.global.pub_sub.lock().await;
    let subscription_receiver = match pub_sub.entry(channel.clone()) {
        Entry::Occupied(e) => e.get().subscribe(),
        Entry::Vacant(e) => {
            let (tx, rx) = tokio::sync::broadcast::channel(32);
            e.insert(tx);
            rx
        }
    };

    // Spawn a task to receive messages and receive cancellation
    let (cancel_sender, cancel_receiver) = tokio::sync::oneshot::channel::<()>();
    let global = context.global.clone();
    let actor_id = context.actor_id.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = subscribe_and_schedule_runs(global, actor_id, handler, context_param, subscription_receiver) => {
                // Noop, just let the task end
            }
            _ = cancel_receiver => {
                debug!("Subscription cancelled");
            }
        }
    });

    context.resources.cancellations.push(Cancellation {
        sender: cancel_sender,
        name: cancellation_name(&channel),
        subject: CancellationSubject::PubSubSubscription,
    });

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn unsubscribe<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let channel = eval_opt_string_param("channel", parameters, context)?;

    if let Some(channel) = channel {
        let name = cancellation_name(&channel);

        let index =
            context.resources.cancellations.iter().position(|c| {
                CancellationSubject::PubSubSubscription == c.subject && c.name == name
            });

        if let Some(i) = index {
            let cancellation = context.resources.cancellations.remove(i);
            cancellation.sender.send(()).unwrap();
        } else {
            return Err(ExecutionError::MissingResource {
                resource_type: "cancellation".into(),
            });
        }
    } else {
        // Unsubscribe all
        let mut index = 0;
        while index < context.resources.cancellations.len() {
            let cancellation = context.resources.cancellations.remove(index);
            if cancellation.subject == CancellationSubject::PubSubSubscription {
                cancellation.sender.send(()).unwrap();
            } else {
                context.resources.cancellations.push(cancellation);
                index += 1;
            }
        }
    }

    Ok(())
}

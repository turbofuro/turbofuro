use std::{collections::HashMap, sync::Arc};

use crate::{
    actor::ActorCommand,
    resources::{Cancellation, CancellationSubject},
};
use deadpool_redis::{Config, Runtime};
use futures_util::StreamExt;
use redis::FromRedisValue;
use tel::{describe, Description, ObjectBody, StorageValue};
use tracing::{debug, instrument};

use crate::{
    actions::as_string,
    errors::ExecutionError,
    evaluations::{eval_optional_param_with_default, eval_param},
    executor::{ExecutionContext, Global, Parameter},
    resources::{RedisPool, Resource},
};

use super::{get_optional_handler_from_parameters, store_value};

fn redis_resource_not_found() -> ExecutionError {
    ExecutionError::MissingResource {
        resource_type: RedisPool::get_type().into(),
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn get_connection<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let connection_string_param = eval_param(
        "connectionString",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    let connection_string = as_string(connection_string_param, "connectionString")?;

    let name_param = eval_optional_param_with_default(
        "name",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::String("default".to_owned()),
    )?;
    let name = as_string(name_param, "name")?;

    // Check if we already have a connection pool with this name
    let exists = { context.global.registry.redis_pools.contains_key(&name) };
    if exists {
        return Ok(());
    }

    // If not let's create a connection pool
    let config = Config::from_url(connection_string);
    let pool =
        config
            .create_pool(Some(Runtime::Tokio1))
            .map_err(|e| ExecutionError::RedisError {
                message: e.to_string(),
            })?;

    debug!("Created Redis connection pool: {}", name);

    let coordinator_handle = setup_pubsub_coordinator(pool.clone(), context.global.clone()).await?;

    // Put to the registry
    context
        .global
        .registry
        .redis_pools
        .insert(name, RedisPool(pool, coordinator_handle));

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn low_level_command<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let connection_name_param = eval_optional_param_with_default(
        "name",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::String("default".to_owned()),
    )?;
    let connection_name = as_string(connection_name_param, "name")?;

    let command_param = eval_param(
        "command",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    let command: String = as_string(command_param, "command")?;

    let args_param = eval_param("args", parameters, &context.storage, &context.environment)?;
    let args = match args_param {
        StorageValue::Array(a) => Ok(a),
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: "args".to_owned(),
            expected: Description::new_base_type("array"),
            actual: describe(s),
        }),
    }?;

    // Retrieve the connection pool
    let redis_pool = {
        context
            .global
            .registry
            .redis_pools
            .get(&connection_name)
            .ok_or(redis_resource_not_found())
            .map(|r| r.value().0.clone())?
    };

    let mut connection = redis_pool
        .get()
        .await
        .map_err(|e| ExecutionError::RedisError {
            message: e.to_string(),
        })?;

    let mut cmd = &mut redis::cmd(&command);
    for arg in args {
        cmd = match arg {
            StorageValue::String(s) => cmd.arg(s),
            StorageValue::Number(f) => cmd.arg(f),
            StorageValue::Boolean(b) => cmd.arg(b),
            StorageValue::Array(v) => cmd.arg(serde_json::to_string(&v).unwrap()),
            StorageValue::Object(v) => cmd.arg(serde_json::to_string(&v).unwrap()),
            StorageValue::Null(_) => cmd.arg("null"),
        };
    }

    let result: RedisResponse =
        cmd.query_async(&mut connection)
            .await
            .map_err(|e| ExecutionError::RedisError {
                message: e.to_string(),
            })?;

    store_value(store_as, context, step_id, result.into())?;
    Ok(())
}

fn cancellation_name(channel: &str) -> String {
    format!("redis_pubsub_{}", channel)
}

#[instrument(level = "trace", skip_all)]
pub async fn subscribe<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let connection_name_param = eval_optional_param_with_default(
        "name",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::String("default".to_owned()),
    )?;
    let connection_name = as_string(connection_name_param, "name")?;

    let channel_param = eval_param(
        "channel",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    let channel: String = as_string(channel_param, "channel")?;

    let handler = get_optional_handler_from_parameters("onMessage", parameters);

    // Retrieve the connection pool
    let mut redis_subscriber = {
        context
            .global
            .registry
            .redis_pools
            .get(&connection_name)
            .ok_or(redis_resource_not_found())
            .map(|r| r.value().1.clone())?
    };

    let cancellation = redis_subscriber
        .subscribe(channel, context.actor_id.clone(), handler)
        .await?;

    context.resources.cancellations.push(cancellation);

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn unsubscribe<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let connection_name_param = eval_optional_param_with_default(
        "name",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::String("default".to_owned()),
    )?;
    let connection_name = as_string(connection_name_param, "name")?;

    let channel_param = eval_param(
        "channel",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    let channel: String = as_string(channel_param, "channel")?;

    let mut redis_subscriber = {
        context
            .global
            .registry
            .redis_pools
            .get(&connection_name)
            .ok_or(redis_resource_not_found())
            .map(|r| r.value().1.clone())?
    };

    redis_subscriber
        .unsubscribe(channel.clone(), context.actor_id.clone())
        .await?;

    context
        .resources
        .cancellations
        .retain(|c| c.name != cancellation_name(&channel));

    Ok(())
}

struct RedisResponse(StorageValue);

impl FromRedisValue for RedisResponse {
    fn from_redis_value(v: &redis::Value) -> redis::RedisResult<Self> {
        match v {
            redis::Value::Nil => Ok(Self(StorageValue::Null(None))),
            redis::Value::Int(i) => Ok(Self(StorageValue::Number(*i as f64))),
            redis::Value::Data(d) => Ok(Self(StorageValue::String(
                String::from_utf8_lossy(d).into(),
            ))),
            redis::Value::Bulk(v) => {
                let mut arr = Vec::new();
                for item in v {
                    arr.push(RedisResponse::from_redis_value(item)?.0);
                }
                Ok(Self(StorageValue::Array(arr)))
            }
            redis::Value::Status(s) => Ok(Self(s.clone().into())),
            redis::Value::Okay => Ok(Self("OK".into())),
        }
    }
}

impl From<RedisResponse> for StorageValue {
    fn from(val: RedisResponse) -> Self {
        val.0
    }
}

#[derive(Debug)]
pub enum RedisPubSubCoordinatorCommand {
    Subscribe {
        channel: String,
        actor_id: String,
        function_ref: Option<String>,
    },
    Unsubscribe {
        channel: String,
        actor_id: String,
    },
}

#[derive(Debug, Clone)]
pub struct RedisPubSubCoordinatorHandle {
    tx: tokio::sync::mpsc::Sender<RedisPubSubCoordinatorCommand>,
}

impl RedisPubSubCoordinatorHandle {
    pub fn new(tx: tokio::sync::mpsc::Sender<RedisPubSubCoordinatorCommand>) -> Self {
        Self { tx }
    }

    pub async fn subscribe(
        &mut self,
        channel: String,
        actor_id: String,
        function_ref: Option<String>,
    ) -> Result<Cancellation, ExecutionError> {
        self.tx
            .send(RedisPubSubCoordinatorCommand::Subscribe {
                channel: channel.clone(),
                actor_id: actor_id.clone(),
                function_ref: function_ref.clone(),
            })
            .await
            .map_err(|e| ExecutionError::RedisError {
                message: e.to_string(),
            })?;

        let (sender, receiver) = tokio::sync::oneshot::channel::<()>();

        let self_clone = self.clone();
        let channel_canceler = channel.clone();
        tokio::spawn(async move {
            let _ = receiver.await; // TODO: Handle error?
            self_clone
                .tx
                .send(RedisPubSubCoordinatorCommand::Unsubscribe {
                    channel: channel_canceler,
                    actor_id: actor_id.clone(),
                })
                .await
                .unwrap();
        });

        Ok(Cancellation {
            sender,
            name: cancellation_name(&channel),
            subject: CancellationSubject::RedisPubSubSubscription,
        })
    }

    pub async fn unsubscribe(
        &mut self,
        channel: String,
        actor_id: String,
    ) -> Result<(), ExecutionError> {
        self.tx
            .send(RedisPubSubCoordinatorCommand::Unsubscribe { channel, actor_id })
            .await
            .map_err(|e| ExecutionError::RedisError {
                message: e.to_string(),
            })?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActorSubscriber {
    id: String,
    function_ref: Option<String>,
}

async fn setup_pubsub_coordinator(
    redis_pool: deadpool_redis::Pool,
    global: Arc<Global>,
) -> Result<RedisPubSubCoordinatorHandle, ExecutionError> {
    let connection = deadpool_redis::Connection::take(redis_pool.get().await.map_err(|e| {
        ExecutionError::RedisError {
            message: e.to_string(),
        }
    })?);
    let mut pubsub: redis::aio::PubSub = connection.into_pubsub();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<RedisPubSubCoordinatorCommand>(100);
    tokio::spawn(async move {
        // Channel -> list of actors
        let mut channels: HashMap<String, Vec<ActorSubscriber>> = HashMap::new();

        loop {
            let mut stream = pubsub.on_message();
            tokio::select! {
                msg = stream.next() => {
                    if msg.is_none() {
                        break;
                    }
                    let msg = msg.unwrap();

                    let channel_name : String =  msg.get_channel_name().into();
                    let message = msg.get_payload::<String>().unwrap().into();
                    let mut storage = ObjectBody::new();
                    storage.insert("message".to_owned(), message);
                    storage.insert("channel".to_owned(), channel_name.clone().into());

                    match channels.get(&channel_name) {
                        Some(v) => {
                            let mut not_founds = Vec::<ActorSubscriber>::new();
                            v.iter().for_each(|sub| {
                                let messenger = {
                                    global
                                    .registry
                                    .actors
                                    .get(&sub.id)
                                    .map(|r| r.value().0.clone())
                                };

                                // Send message
                                if let Some(messenger) = messenger {
                                    let storage = storage.clone();
                                    let function_ref = sub.function_ref.clone();
                                    tokio::spawn(async move {
                                        if let Some(ref function_ref) = function_ref {
                                            let _ = messenger.send(ActorCommand::RunFunctionRef {
                                                function_ref: function_ref.clone(),
                                                storage,
                                                sender: None,
                                            }).await;
                                        } else {
                                            let _ = messenger.send(ActorCommand::Run {
                                                handler: "onMessage".to_owned(),
                                                storage,
                                                sender: None,
                                            }).await;
                                        }
                                    });

                                } else {
                                    debug!("Alarm fired but actor {} was not found", sub.id);
                                    not_founds.push(sub.clone());
                                }
                            });

                            not_founds.iter().for_each(|sub| {
                                debug!("Alarm fired but actor {} was not found, cleaning up", sub.id);
                                channels.get_mut(&channel_name).unwrap().retain(|a| a.id != sub.id);
                            });
                        }
                        None => {
                            debug!("Unsubscribe on channel without any actors, channel was: {}", channel_name);
                        }
                    }

                }
                cmd = rx.recv() => {
                    drop(stream);
                    if let Some(cmd) = cmd {
                        match cmd {
                            RedisPubSubCoordinatorCommand::Subscribe { channel, actor_id, function_ref } => {
                                match channels.get_mut(&channel) {
                                    Some(v) => {
                                        v.push(ActorSubscriber {
                                            id: actor_id.clone(),
                                            function_ref: function_ref.clone(),
                                        });
                                        if v.len() == 1 { // In case an empty array was left
                                            pubsub.subscribe(channel).await.unwrap();
                                        }
                                    }
                                    None => {
                                        channels.insert(channel.clone(), vec![ActorSubscriber {
                                            id: actor_id.clone(),
                                            function_ref: function_ref.clone(),
                                        }]);
                                        pubsub.subscribe(channel).await.unwrap();
                                    }
                                }
                            }
                            RedisPubSubCoordinatorCommand::Unsubscribe { channel, actor_id} => {
                                match channels.get_mut(&channel) {
                                    Some(v) => {
                                        v.retain(|a| a.id != actor_id);
                                        if v.is_empty() {
                                            channels.remove(&channel);
                                            pubsub.unsubscribe(channel).await.unwrap();
                                        }
                                    }
                                    None => {
                                        debug!("Unsubscribe on channel without any actors, channel was: {}", channel);
                                        pubsub.unsubscribe(channel).await.unwrap();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    Ok(RedisPubSubCoordinatorHandle::new(tx))
}

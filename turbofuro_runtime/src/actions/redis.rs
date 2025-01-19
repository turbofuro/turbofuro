use std::{collections::HashMap, fmt::Display, sync::Arc, time::Duration};

use crate::{
    actor::ActorCommand,
    evaluations::{eval_opt_string_param, eval_string_param, get_optional_handler_from_parameters},
    resources::{generate_resource_id, Cancellation, CancellationSubject},
};
use deadpool_redis::{Config, Runtime};
use futures_util::StreamExt;
use redis::FromRedisValue;
use tel::{describe, Description, ObjectBody, StorageValue, NULL};
use tokio::time::sleep;
use tracing::{debug, error, instrument, warn};

use crate::{
    errors::ExecutionError,
    evaluations::eval_param,
    executor::{ExecutionContext, Global, Parameter},
    resources::{RedisPool, Resource},
};

use super::store_value;

#[instrument(level = "trace", skip_all)]
pub async fn get_connection<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let connection_string = eval_string_param("connectionString", parameters, context)?;
    let name = eval_opt_string_param("name", parameters, context)?.unwrap_or("default".to_owned());

    // Check if we already have a connection pool with this name
    let exists = { context.global.registry.redis_pools.contains_key(&name) };
    if exists {
        return Ok(());
    }

    // If not let's create a connection pool
    let config = Config::from_url(&connection_string);
    let pool =
        config
            .create_pool(Some(Runtime::Tokio1))
            .map_err(|e| ExecutionError::RedisError {
                message: e.to_string(),
            })?;

    debug!("Created Redis connection pool: {}", name);

    let mut connection = pool.get().await.map_err(|e| ExecutionError::RedisError {
        message: e.to_string(),
    })?;

    // Ping the connection to make sure it's working
    let _: String = redis::cmd("PING")
        .query_async(&mut connection)
        .await
        .map_err(|e| ExecutionError::RedisError {
            message: e.to_string(),
        })?;

    // Setup the pubsub coordinator
    let coordinator_handle =
        setup_pubsub_coordinator(&connection_string, context.global.clone()).await?;

    // Put to the registry
    let pool_id = generate_resource_id();
    context
        .global
        .registry
        .redis_pools
        .insert(name.clone(), RedisPool(pool_id, pool, coordinator_handle));
    context
        .note_named_resource_provisioned(pool_id, RedisPool::static_type(), name)
        .await;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn low_level_command<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let name = eval_opt_string_param("name", parameters, context)?.unwrap_or("default".to_owned());
    let command = eval_string_param("command", parameters, context)?;

    let args_param = eval_param("args", parameters, context)?;
    let args = match args_param {
        StorageValue::Array(a) => Ok(a),
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: "args".to_owned(),
            expected: Description::new_base_type("array"),
            actual: describe(s),
        }),
    }?;

    // Retrieve the connection pool
    let (pool_id, redis_pool) = {
        context
            .global
            .registry
            .redis_pools
            .get(&name)
            .ok_or_else(RedisPool::missing)
            .map(|r| (r.value().0, r.value().1.clone()))?
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

    context
        .note_named_resource_used(pool_id, RedisPool::static_type(), name)
        .await;

    store_value(store_as, context, step_id, result.into()).await?;
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
    let name = eval_opt_string_param("name", parameters, context)?.unwrap_or("default".to_owned());
    let channel = eval_string_param("channel", parameters, context)?;

    let handler = get_optional_handler_from_parameters("onMessage", parameters);

    // Retrieve the connection pool
    let (pool_id, mut redis_subscriber) = {
        context
            .global
            .registry
            .redis_pools
            .get(&name)
            .ok_or_else(RedisPool::missing)
            .map(|r| (r.value().0, r.value().2.clone()))?
    };

    let cancellation = redis_subscriber
        .subscribe(channel, context.actor_id.clone(), handler)
        .await?;
    context
        .note_named_resource_used(pool_id, RedisPool::static_type(), name)
        .await;
    context
        .note_resource_provisioned(cancellation.id, Cancellation::static_type())
        .await;
    context.resources.add_cancellation(cancellation);

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn unsubscribe<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let name = eval_opt_string_param("name", parameters, context)?.unwrap_or("default".to_owned());
    let channel = eval_string_param("channel", parameters, context)?;

    let (pool_id, mut redis_subscriber) = {
        context
            .global
            .registry
            .redis_pools
            .get(&name)
            .ok_or_else(RedisPool::missing)
            .map(|r| (r.value().0, r.value().2.clone()))?
    };

    redis_subscriber
        .unsubscribe(channel.clone(), context.actor_id.clone())
        .await?;

    context
        .note_named_resource_used(pool_id, RedisPool::static_type(), name.clone())
        .await;

    while let Some(cancellation) = context.resources.pop_cancellation_where(|c| {
        matches!(c.subject, CancellationSubject::RedisPubSubSubscription) && c.name == name
    }) {
        context
            .note_resource_consumed(cancellation.id, Cancellation::static_type())
            .await;
        drop(cancellation);
    }

    Ok(())
}

enum RedisValueParseError {
    Unsupported,
    Invalid,
}

impl Display for RedisValueParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RedisValueParseError::Unsupported => write!(f, "Unsupported value"),
            RedisValueParseError::Invalid => write!(f, "Invalid value"),
        }
    }
}

struct RedisResponse(StorageValue);

impl TryFrom<redis::Value> for RedisResponse {
    type Error = RedisValueParseError;

    fn try_from(value: redis::Value) -> Result<Self, Self::Error> {
        let value = match value {
            redis::Value::Nil => NULL,
            redis::Value::Int(i) => StorageValue::Number(i as f64),
            redis::Value::BulkString(vec) => {
                StorageValue::String(String::from_utf8_lossy(&vec).into())
            }
            redis::Value::Array(vec) => {
                let mut arr = Vec::new();
                for item in vec {
                    arr.push(RedisResponse::try_from(item)?.0);
                }
                StorageValue::Array(arr)
            }
            redis::Value::SimpleString(s) => StorageValue::String(s.clone()),
            redis::Value::Okay => StorageValue::String("OK".to_owned()),
            redis::Value::Map(vec) => {
                let mut obj = ObjectBody::new();
                for (key, value) in vec {
                    let key = RedisResponse::try_from(key)?.0;
                    let key = key.to_string().map_err(|_| RedisValueParseError::Invalid)?;
                    obj.insert(key, RedisResponse::try_from(value)?.0);
                }
                StorageValue::Object(obj)
            }
            redis::Value::Attribute { data, attributes } => {
                // TODO: Validate this implementation
                let mut obj = ObjectBody::new();
                obj.insert("data".to_owned(), RedisResponse::try_from(*data)?.0);
                for (key, value) in attributes {
                    let key = RedisResponse::try_from(key)?.0;
                    let key = key.to_string().map_err(|_| RedisValueParseError::Invalid)?;
                    obj.insert(key, RedisResponse::try_from(value)?.0);
                }
                StorageValue::Object(obj)
            }
            redis::Value::Set(vec) => {
                let mut arr = Vec::new();
                for item in vec {
                    arr.push(RedisResponse::try_from(item)?.0);
                }
                StorageValue::Array(arr)
            }
            redis::Value::Double(d) => StorageValue::Number(d),
            redis::Value::Boolean(b) => StorageValue::Boolean(b),
            redis::Value::VerbatimString { format: _, text } => StorageValue::String(text.clone()),
            redis::Value::BigNumber(_big_int) => {
                return Err(RedisValueParseError::Unsupported);
            }
            redis::Value::Push { .. } => {
                return Err(RedisValueParseError::Unsupported);
            }
            redis::Value::ServerError(_server_error) => {
                return Err(RedisValueParseError::Unsupported);
            }
        };

        Ok(Self(value))
    }
}

impl FromRedisValue for RedisResponse {
    fn from_redis_value(v: &redis::Value) -> redis::RedisResult<Self> {
        RedisResponse::try_from(v.clone()).map_err(|e| {
            redis::RedisError::from((
                redis::ErrorKind::ParseError,
                "Could not parse value",
                e.to_string(),
            ))
        })
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
        sender: tokio::sync::oneshot::Sender<Result<(), ExecutionError>>,
    },
    Unsubscribe {
        channel: String,
        actor_id: String,
        sender: tokio::sync::oneshot::Sender<Result<(), ExecutionError>>,
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
        let (sender, receiver) = tokio::sync::oneshot::channel::<Result<(), ExecutionError>>();
        self.tx
            .send(RedisPubSubCoordinatorCommand::Subscribe {
                channel: channel.clone(),
                actor_id: actor_id.clone(),
                function_ref: function_ref.clone(),
                sender,
            })
            .await
            .map_err(|e| ExecutionError::RedisError {
                message: format!("Subscribe sender failed: {}", e),
            })?;

        receiver.await.map_err(|e| ExecutionError::RedisError {
            message: format!("Subscribe receiver failed: {}", e),
        })??;

        let (sender, receiver) = tokio::sync::oneshot::channel::<()>();
        let self_clone = self.clone();
        let channel_copy = channel.clone();

        // Spawn cancellation task
        tokio::spawn(async move {
            match receiver.await {
                Ok(_) => {
                    debug!(
                        "Cancellation received for Redis PubSub subscription on channel {}",
                        channel_copy
                    );
                }
                Err(_) => {
                    debug!(
                        "Cancellation sender dropped for Redis PubSub subscription on channel {}",
                        channel_copy
                    );
                }
            }

            let (sender, receiver) = tokio::sync::oneshot::channel::<Result<(), ExecutionError>>();
            match self_clone
                .tx
                .send(RedisPubSubCoordinatorCommand::Unsubscribe {
                    channel: channel_copy.clone(),
                    actor_id: actor_id.clone(),
                    sender,
                })
                .await
            {
                Ok(_) => {
                    match receiver.await.map_err(|e| ExecutionError::RedisError {
                        message: e.to_string(),
                    }) {
                        Ok(_) => {
                            debug!("Unsubscribed from Redis PubSub channel {}", channel_copy);
                        }
                        Err(e) => {
                            error!(
                                "Failed to unsubscribe from Redis PubSub channel {}: {:?}",
                                channel_copy, e
                            );
                        }
                    }
                }
                Err(e) => {
                    error!(
                        "Failed to unsubscribe from Redis PubSub channel {}: {:?}",
                        channel_copy, e
                    );
                }
            }
        });

        Ok(Cancellation {
            id: generate_resource_id(),
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
        let (sender, receiver) = tokio::sync::oneshot::channel::<Result<(), ExecutionError>>();
        self.tx
            .send(RedisPubSubCoordinatorCommand::Unsubscribe {
                channel,
                actor_id,
                sender,
            })
            .await
            .map_err(|e| ExecutionError::RedisError {
                message: e.to_string(),
            })?;

        receiver.await.map_err(|e| ExecutionError::RedisError {
            message: e.to_string(),
        })??;

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActorSubscriber {
    id: String,
    function_ref: Option<String>,
}

async fn setup_pubsub_coordinator(
    connection_string: &str,
    global: Arc<Global>,
) -> Result<RedisPubSubCoordinatorHandle, ExecutionError> {
    // Let's establish a separate connection for the PubSub coordinator
    // This is a limitation of the current Rust Redis client, but it's not a big deal
    let client = redis::Client::open(connection_string).unwrap();
    let mut pubsub = client.get_async_pubsub().await?;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<RedisPubSubCoordinatorCommand>(100);
    tokio::spawn(async move {
        // Map of channels to list of actors
        let mut channels = HashMap::<String, Vec<ActorSubscriber>>::new();
        loop {
            let mut stream = pubsub.on_message();
            tokio::select! {
                msg = stream.next() => {
                    let msg = match msg {
                        Some(msg) => msg,
                        None => {
                            // This probably means our connection was closed
                            match client.get_async_pubsub().await {
                                Ok(connection) => {
                                    // Ok we got a new connection, let's re-assign it
                                    drop(stream);
                                    pubsub = connection;

                                    // Re-subscribe
                                    for (channel, i) in channels.iter() {
                                        warn!("Re-subscribing to channel {:?} {:?}", channel, i);
                                        match pubsub.subscribe(channel).await {
                                            Ok(_) => {
                                                debug!("Re-subscribed to channel {}", channel);
                                            }
                                            Err(err) => {
                                                warn!("Failed to re-subscribe to channel: {}", err);
                                            }
                                        }
                                    }
                                },
                                Err(e) => {
                                    warn!("Failed to reconnect to Redis PubSub: {}", e);
                                    sleep(Duration::from_secs(5)).await; // Wait 5 seconds before retrying
                                }
                            }
                            continue;
                        }
                    };

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
                                    .map(|r| r.value().clone())
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
                                                references: HashMap::new(),
                                                sender: None,
                                                execution_id: None,
                                            }).await;
                                        } else {
                                            let _ = messenger.send(ActorCommand::Run {
                                                handler: "onMessage".to_owned(),
                                                storage,
                                                references: HashMap::new(),
                                                sender: None,
                                                execution_id: None,
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
                msg = rx.recv() => {
                    drop(stream);
                    match msg {
                        Some(cmd) => match cmd {
                            RedisPubSubCoordinatorCommand::Subscribe { channel, actor_id, function_ref, sender } => {
                                debug!("Subscribing to channel: {} for actor ID {}", channel, actor_id);
                                match channels.get_mut(&channel) {
                                    Some(v) => {
                                        v.push(ActorSubscriber {
                                            id: actor_id.clone(),
                                            function_ref: function_ref.clone(),
                                        });

                                        // In case an empty array was left, re-subscribe
                                        // Though I should probably learn how this thing works
                                        if v.len() == 1 {
                                            match pubsub.subscribe(channel).await {
                                                Ok(_) => {
                                                    let _ = sender.send(Ok(()));
                                                }
                                                Err(err) => {
                                                    error!("Failed to subscribe to channel: {}", err);
                                                    let _ = sender.send(Err(err.into()));
                                                }
                                            }
                                        } else {
                                            let _ = sender.send(Ok(()));
                                        }
                                    }
                                    None => {
                                        channels.insert(channel.clone(), vec![ActorSubscriber {
                                            id: actor_id.clone(),
                                            function_ref: function_ref.clone(),
                                        }]);
                                        match pubsub.subscribe(channel).await {
                                            Ok(_) => {
                                                let _ = sender.send(Ok(()));
                                            }
                                            Err(err) => {
                                                error!("Failed to subscribe to channel (2): {}", err);
                                                let _ = sender.send(Err(err.into()));
                                            }
                                        }
                                    }
                                }
                            }
                            RedisPubSubCoordinatorCommand::Unsubscribe { channel, actor_id, sender } => {
                                debug!("Unsubscribing to channel: {} for actor ID {}", channel, actor_id);
                                match channels.get_mut(&channel) {
                                    Some(v) => {
                                        v.retain(|a| a.id != actor_id);

                                        // When it's empty, unsubscribe
                                        if v.is_empty() {
                                            channels.remove(&channel);
                                            match pubsub.unsubscribe(channel).await {
                                                Ok(_) => {
                                                    let _ = sender.send(Ok(()));
                                                }
                                                Err(err) => {
                                                    error!("Failed to unsubscribe from channel: {}", err);
                                                    let _ = sender.send(Err(err.into()));
                                                }
                                            }
                                        } else {
                                            let _ = sender.send(Ok(()));
                                        }
                                    }
                                    None => {
                                        debug!("Unsubscribe on channel without any actors, channel was: {}", channel);
                                        match pubsub.unsubscribe(channel).await {
                                            Ok(_) => {
                                                let _ = sender.send(Ok(()));
                                            }
                                            Err(err) => {
                                                error!("Failed to unsubscribe from channel (2): {}", err);
                                                let _ = sender.send(Err(err.into()));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        None => {
                            // If all handles are dropped, we should stop the coordinator
                            debug!("Redis PubSub coordinator dropped");
                            break;
                        }
                    }
                }
            }
        }
        debug!("Redis PubSub coordinator ended");
    });

    Ok(RedisPubSubCoordinatorHandle::new(tx))
}

#[instrument(level = "trace", skip_all)]
pub async fn drop_connection<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let name = eval_opt_string_param("name", parameters, context)?.unwrap_or("default".to_owned());

    let (name, pool) = context
        .global
        .registry
        .redis_pools
        .remove(&name)
        .ok_or_else(|| ExecutionError::RedisError {
            message: "Redis pool not found".to_owned(),
        })?;

    context
        .note_named_resource_consumed(pool.0, RedisPool::static_type(), name)
        .await;

    Ok(())
}

use tel::StorageValue;

use crate::{errors::ExecutionError, evaluations::eval_selector, executor::ExecutionContext};

pub mod actors;
pub mod alarms;
pub mod convert;
pub mod crypto;
pub mod debug;
pub mod fantoccini;
pub mod fs;
pub mod http_client;
pub mod http_server;
pub mod image;
pub mod kv;
pub mod libsql;
pub mod lua;
pub mod mail;
pub mod mustache;
pub mod ollama;
pub mod os;
pub mod postgres;
pub mod pubsub;
pub mod redis;
pub mod regex;
pub mod sound;
pub mod tasks;
pub mod time;
pub mod wasm;
pub mod websocket_server;

pub async fn store_value(
    store_as: Option<&str>,
    context: &mut ExecutionContext<'_>,
    step_id: &str,
    value: StorageValue,
) -> Result<(), ExecutionError> {
    if let Some(expression) = store_as {
        let selector = eval_selector(expression, &context.storage, &context.environment)?;
        context.add_to_storage(step_id, selector, value).await?;
    }
    Ok(())
}

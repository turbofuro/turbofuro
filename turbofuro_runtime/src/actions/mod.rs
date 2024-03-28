use std::collections::HashMap;

use tel::{describe, Description, StorageValue};

use crate::{
    errors::ExecutionError,
    evaluations::eval_selector,
    executor::{ExecutionContext, Parameter},
};

pub mod actors;
pub mod alarms;
pub mod convert;
pub mod crypto;
pub mod fs;
pub mod http_client;
pub mod http_server;
pub mod kv;
pub mod os;
pub mod postgres;
pub mod pubsub;
pub mod redis;
pub mod time;
pub mod wasm;
pub mod websocket;

pub fn store_value(
    store_as: Option<&str>,
    context: &mut ExecutionContext<'_>,
    step_id: &str,
    value: StorageValue,
) -> Result<(), ExecutionError> {
    if let Some(expression) = store_as {
        let selector = eval_selector(expression, &context.storage, &context.environment)?;
        context.add_to_storage(step_id, selector, value)?;
    }
    Ok(())
}

pub fn as_string(s: StorageValue, name: &str) -> Result<String, ExecutionError> {
    match s {
        StorageValue::String(s) => Ok(s),
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: name.to_owned(),
            expected: Description::new_base_type("string"),
            actual: describe(s),
        }),
    }
}

pub fn as_integer(s: StorageValue, name: &str) -> Result<i32, ExecutionError> {
    match s {
        StorageValue::Number(f) => Ok(f as i32),
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: name.to_owned(),
            expected: Description::new_base_type("number"),
            actual: describe(s),
        }),
    }
}

pub fn get_handlers_from_parameters(parameters: &Vec<Parameter>) -> HashMap<String, String> {
    let mut handlers: HashMap<String, String> = HashMap::new();
    for parameter in parameters {
        match parameter {
            Parameter::Tel { .. } => {}
            Parameter::FunctionRef { name, id } => {
                if name.starts_with("on") {
                    handlers.insert(name.to_owned(), id.into());
                }
            }
        }
    }
    handlers
}

pub fn get_optional_handler_from_parameters(
    parameter_name: &str,
    parameters: &[Parameter],
) -> Option<String> {
    parameters
        .iter()
        .find(|p| match p {
            Parameter::FunctionRef { name, id: _ } => name == parameter_name,
            _ => false,
        })
        .map(|p| match p {
            Parameter::FunctionRef { id, .. } => id.clone(),
            _ => unreachable!(),
        })
}

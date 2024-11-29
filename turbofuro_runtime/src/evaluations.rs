use std::collections::HashMap;

use memoize::memoize;
use num::ToPrimitive;
use tel::{
    describe, parse_description, Description, ObjectBody, ParseResult, Selector, SelectorPart,
    Storage, StorageValue, TelError,
};

use crate::{
    errors::ExecutionError,
    executor::{Environment, ExecutionContext, Parameter},
};

#[memoize]
fn parse_memoized(expression: String) -> ParseResult {
    tel::parse(&expression)
}

pub fn eval_param(
    parameter_name: &str,
    parameters: &Vec<Parameter>,
    storage: &ObjectBody,
    environment: &Environment,
) -> Result<StorageValue, ExecutionError> {
    for parameter in parameters {
        match parameter {
            Parameter::Tel { name, expression } => {
                if name == parameter_name {
                    let value = eval(expression, storage, environment)?;
                    return Ok(value);
                }
            }
            Parameter::FunctionRef { .. } => {}
        }
    }
    Err(ExecutionError::MissingParameter {
        name: parameter_name.to_string(),
    })
}

pub fn eval_optional_param(
    parameter_name: &str,
    parameters: &Vec<Parameter>,
    storage: &ObjectBody,
    environment: &Environment,
) -> Result<Option<StorageValue>, ExecutionError> {
    match eval_param(parameter_name, parameters, storage, environment) {
        Ok(value) => match value {
            StorageValue::Null(_) => Ok(None),
            value => Ok(Some(value)),
        },
        Err(e) => match e {
            ExecutionError::MissingParameter { .. } => Ok(None),
            e => Err(e),
        },
    }
}

pub fn eval_optional_param_with_default(
    parameter_name: &str,
    parameters: &Vec<Parameter>,
    storage: &ObjectBody,
    environment: &Environment,
    default: StorageValue,
) -> Result<StorageValue, ExecutionError> {
    match eval_param(parameter_name, parameters, storage, environment) {
        Ok(value) => match value {
            StorageValue::Null(_) => Ok(default),
            value => Ok(value),
        },
        Err(e) => match e {
            ExecutionError::MissingParameter { .. } => Ok(default),
            e => Err(e),
        },
    }
}

pub fn eval_selector_param(
    parameter_name: &str,
    parameters: &Vec<Parameter>,
    storage: &mut ObjectBody,
    environment: &Environment,
) -> Result<Vec<SelectorPart>, ExecutionError> {
    for parameter in parameters {
        match parameter {
            Parameter::Tel { name, expression } => {
                if name == parameter_name {
                    return eval_selector(expression, storage, environment);
                }
            }
            Parameter::FunctionRef { .. } => unimplemented!(),
        }
    }
    Err(ExecutionError::MissingParameter {
        name: parameter_name.to_string(),
    })
}
pub fn eval<T: Storage>(
    expression: &str,
    storage: &T,
    environment: &Environment,
) -> Result<StorageValue, ExecutionError> {
    let result = parse_memoized(expression.to_owned());
    if !result.errors.is_empty() {
        return Err(TelError::ParseError {
            errors: result.errors,
        }
        .into());
    }
    let expr = result.expr.ok_or(ExecutionError::Unknown {
        message: "No expression".into(),
    })?;
    let evaluated =
        tel::evaluate_value(expr, storage, &environment.variables).map_err(ExecutionError::from)?;

    Ok(evaluated)
}

pub fn eval_selector<'a>(
    expression: &str,
    storage: &'a ObjectBody,
    environment: &'a Environment,
) -> Result<Selector, ExecutionError> {
    let result = parse_memoized(expression.to_owned());
    if !result.errors.is_empty() {
        return Err(TelError::ParseError {
            errors: result.errors,
        }
        .into());
    }
    let expr = result.expr.ok_or(ExecutionError::Unknown {
        message: "No expression".into(),
    })?;
    let evaluated = tel::evaluate_selector(expr, storage, &environment.variables)
        .map_err(ExecutionError::from)?;

    Ok(evaluated)
}

pub fn eval_description(description: &str) -> Result<Description, ExecutionError> {
    let result = parse_description(description);
    if !result.errors.is_empty() {
        return Err(TelError::ParseError {
            errors: result.errors,
        }
        .into());
    }
    let expr = result.expr.ok_or(ExecutionError::Unknown {
        message: "No expression".into(),
    })?;

    let evaluated = tel::evaluate_description_notation(expr).map_err(ExecutionError::from)?;
    Ok(evaluated)
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

pub fn as_byte_array(s: StorageValue, name: &str) -> Result<Vec<u8>, ExecutionError> {
    match s {
        StorageValue::String(s) => Ok(s.as_bytes().to_vec()),
        StorageValue::Array(arr) => {
            let mut result = Vec::new();
            for (i, s) in arr.into_iter().enumerate() {
                match s {
                    StorageValue::Number(n) => match n.to_u8() {
                        Some(b) => result.push(b),
                        None => {
                            return Err(ExecutionError::ParameterInvalid {
                                name: name.to_owned(),
                                message: "Could not convert number to byte".to_owned(),
                            })
                        }
                    },
                    _ => {
                        return Err(ExecutionError::ParameterTypeMismatch {
                            name: format!("{}[{}]", name, i),
                            expected: Description::new_base_type("number"),
                            actual: describe(s),
                        });
                    }
                }
            }
            Ok(result)
        }
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: name.to_owned(),
            expected: Description::Union {
                of: vec![
                    Description::new_base_type("array"),
                    Description::new_base_type("string"),
                ],
            },
            actual: describe(s),
        }),
    }
}

pub fn as_string_or_array_string(
    s: StorageValue,
    name: &str,
) -> Result<Vec<String>, ExecutionError> {
    match s {
        StorageValue::String(s) => Ok(vec![s]),
        StorageValue::Array(arr) => {
            let mut result = Vec::new();
            for s in arr {
                match s {
                    StorageValue::String(s) => result.push(s),
                    s => {
                        return Err(ExecutionError::ParameterTypeMismatch {
                            name: name.to_owned(),
                            expected: Description::new_base_type("string"),
                            actual: describe(s),
                        })
                    }
                }
            }
            Ok(result)
        }
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: name.to_owned(),
            expected: Description::new_base_type("string"),
            actual: describe(s),
        }),
    }
}

pub fn as_integer(s: StorageValue, name: &str) -> Result<i64, ExecutionError> {
    match s {
        StorageValue::Number(f) => Ok(f as i64),
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: name.to_owned(),
            expected: Description::new_base_type("number"),
            actual: describe(s),
        }),
    }
}

pub fn as_boolean(s: StorageValue, name: &str) -> Result<bool, ExecutionError> {
    match s {
        StorageValue::Boolean(b) => Ok(b),
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: name.to_owned(),
            expected: Description::new_base_type("boolean"),
            actual: describe(s),
        }),
    }
}

pub fn as_number(s: StorageValue, name: &str) -> Result<f64, ExecutionError> {
    match s {
        StorageValue::Number(s) => Ok(s),
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: name.to_owned(),
            expected: Description::new_base_type("number"),
            actual: describe(s),
        }),
    }
}

pub fn as_u64(s: StorageValue, name: &str) -> Result<u64, ExecutionError> {
    match s {
        StorageValue::Number(f) => Ok(f as u64),
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

pub fn get_handler_from_parameters(
    parameter_name: &str,
    parameters: &[Parameter],
) -> Result<String, ExecutionError> {
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
        .ok_or_else(|| ExecutionError::MissingParameter {
            name: parameter_name.to_owned(),
        })
}

pub fn eval_string_param(
    name: &str,
    parameters: &Vec<Parameter>,
    context: &ExecutionContext<'_>,
) -> Result<String, ExecutionError> {
    let value = eval_param(name, parameters, &context.storage, &context.environment)?;
    as_string(value, name)
}

pub fn eval_opt_string_param(
    name: &str,
    parameters: &Vec<Parameter>,
    context: &ExecutionContext<'_>,
) -> Result<Option<String>, ExecutionError> {
    let value = eval_optional_param(name, parameters, &context.storage, &context.environment)?;
    match value {
        Some(value) => as_string(value, name).map(|s| Some(s)),
        None => Ok(None),
    }
}

pub fn eval_integer_param(
    name: &str,
    parameters: &Vec<Parameter>,
    context: &ExecutionContext<'_>,
) -> Result<i64, ExecutionError> {
    let value = eval_param(name, parameters, &context.storage, &context.environment)?;
    as_integer(value, name)
}

pub fn eval_opt_integer_param(
    name: &str,
    parameters: &Vec<Parameter>,
    context: &ExecutionContext<'_>,
) -> Result<Option<i64>, ExecutionError> {
    let value = eval_optional_param(name, parameters, &context.storage, &context.environment)?;
    match value {
        Some(value) => as_integer(value, name).map(|i| Some(i)),
        None => Ok(None),
    }
}

pub fn eval_boolean_param(
    name: &str,
    parameters: &Vec<Parameter>,
    context: &ExecutionContext<'_>,
) -> Result<bool, ExecutionError> {
    let value = eval_param(name, parameters, &context.storage, &context.environment)?;
    as_boolean(value, name)
}

pub fn eval_opt_boolean_param(
    name: &str,
    parameters: &Vec<Parameter>,
    context: &ExecutionContext<'_>,
) -> Result<Option<bool>, ExecutionError> {
    let value = eval_optional_param(name, parameters, &context.storage, &context.environment)?;
    match value {
        Some(value) => as_boolean(value, name).map(|i| Some(i)),
        None => Ok(None),
    }
}

pub fn eval_u64_param(
    name: &str,
    parameters: &Vec<Parameter>,
    context: &ExecutionContext<'_>,
) -> Result<u64, ExecutionError> {
    let value = eval_param(name, parameters, &context.storage, &context.environment)?;
    as_u64(value, name)
}

pub fn eval_opt_u64_param(
    name: &str,
    parameters: &Vec<Parameter>,
    context: &ExecutionContext<'_>,
) -> Result<Option<u64>, ExecutionError> {
    let value = eval_optional_param(name, parameters, &context.storage, &context.environment)?;
    match value {
        Some(value) => as_u64(value, name).map(|i| Some(i)),
        None => Ok(None),
    }
}

pub fn eval_number_param(
    name: &str,
    parameters: &Vec<Parameter>,
    context: &ExecutionContext<'_>,
) -> Result<f64, ExecutionError> {
    let value = eval_param(name, parameters, &context.storage, &context.environment)?;
    as_number(value, name)
}

pub fn eval_opt_number_param(
    name: &str,
    parameters: &Vec<Parameter>,
    context: &ExecutionContext<'_>,
) -> Result<Option<f64>, ExecutionError> {
    let value = eval_optional_param(name, parameters, &context.storage, &context.environment)?;
    match value {
        Some(value) => as_number(value, name).map(|i| Some(i)),
        None => Ok(None),
    }
}

pub fn eval_byte_array_param(
    name: &str,
    parameters: &Vec<Parameter>,
    context: &ExecutionContext<'_>,
) -> Result<Vec<u8>, ExecutionError> {
    let value = eval_param(name, parameters, &context.storage, &context.environment)?;
    as_byte_array(value, name)
}

use memoize::memoize;
use tel::{ObjectBody, ParseResult, Selector, SelectorPart, StorageValue, TelError};

use crate::{
    errors::ExecutionError,
    executor::{Environment, Parameter},
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
pub fn eval(
    expression: &str,
    storage: &ObjectBody,
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

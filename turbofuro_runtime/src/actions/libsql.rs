use libsql::params::IntoValue;
use libsql::{params_from_iter, Error};
use std::collections::HashMap;
use tel::{describe, Description, StorageValue, NULL};
use tracing::{debug, instrument, warn};
use url::Url;

use crate::evaluations::{eval_opt_string_param, eval_string_param};
use crate::resources::generate_resource_id;
use crate::{
    errors::ExecutionError,
    evaluations::eval_optional_param_with_default,
    executor::{ExecutionContext, Parameter},
    resources::{LibSql, Resource},
};

use super::store_value;

impl From<Error> for ExecutionError {
    fn from(error: Error) -> Self {
        ExecutionError::LibSqlError {
            message: error.to_string(),
            stage: "unknown".into(),
        }
    }
}

pub struct LibSqlValue(StorageValue);

impl From<LibSqlValue> for StorageValue {
    fn from(value: LibSqlValue) -> Self {
        value.0
    }
}

impl From<StorageValue> for LibSqlValue {
    fn from(value: StorageValue) -> Self {
        LibSqlValue(value)
    }
}

fn parse_libsql_value(value: libsql::Value) -> StorageValue {
    match value {
        libsql::Value::Null => NULL,
        libsql::Value::Integer(i) => (i as f64).into(),
        libsql::Value::Real(f) => f.into(),
        libsql::Value::Text(s) => s.into(),
        libsql::Value::Blob(vec) => StorageValue::new_byte_array(&vec),
    }
}

impl IntoValue for LibSqlValue {
    fn into_value(self) -> Result<libsql::Value, libsql::Error> {
        match self.0 {
            StorageValue::String(s) => Ok(libsql::Value::Text(s)),
            StorageValue::Number(f) => Ok(libsql::Value::Real(f)),
            StorageValue::Boolean(b) => Ok(libsql::Value::Integer(b as i64)),
            StorageValue::Array(arr) => {
                let json = serde_json::to_string(&arr).unwrap();
                Ok(libsql::Value::Text(json))
            }
            StorageValue::Object(obj) => {
                let json = serde_json::to_string(&obj).unwrap();
                Ok(libsql::Value::Text(json))
            }
            StorageValue::Null(_) => Ok(libsql::Value::Null),
        }
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn get_connection<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let connection_string = eval_string_param("connectionString", parameters, context)?;
    let name = eval_opt_string_param("name", parameters, context)?.unwrap_or("default".to_owned());

    // Check if we already have a connection pool with this name
    let exists = { context.global.registry.libsql.contains_key(&name) };
    if exists {
        return Ok(());
    }

    let url = Url::parse(&connection_string).map_err(|e| ExecutionError::ParameterInvalid {
        name: "connectionString".into(),
        message: e.to_string(),
    })?;

    let db = match url.scheme() {
        "file" => {
            let file_path = url
                .to_file_path()
                .map_err(|_| ExecutionError::ParameterInvalid {
                    name: "connectionString".into(),
                    message: "Could not parse file path".into(),
                })?;

            debug!("Opening local file: {}", file_path.display());

            libsql::Builder::new_local(file_path)
                .build()
                .await
                .map_err(|e| ExecutionError::LibSqlError {
                    message: e.to_string(),
                    stage: "db_creation_local".into(),
                })?
        }
        _ => {
            return Err(ExecutionError::ParameterInvalid {
                name: "connectionString".into(),
                message: "Only file:// URLs are supported".into(),
            })
        }
    };

    let connection = db.connect().map_err(|e| ExecutionError::LibSqlError {
        message: e.to_string(),
        stage: "connection_creation".into(),
    })?;

    // Put to the registry
    let connection_id = generate_resource_id();
    context
        .global
        .registry
        .libsql
        .insert(name.clone(), LibSql(connection_id, connection));

    context
        .note_named_resource_provisioned(connection_id, LibSql::static_type(), name)
        .await;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn query<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let name = eval_opt_string_param("name", parameters, context)?.unwrap_or("default".to_owned());
    let statement = eval_string_param("statement", parameters, context)?;

    let params_param = eval_optional_param_with_default(
        "params",
        parameters,
        context,
        StorageValue::Array(vec![]),
    )?;
    let p: Vec<LibSqlValue> = match params_param {
        StorageValue::Array(v) => Ok(v.into_iter().map(LibSqlValue).collect()),
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: "params".to_owned(),
            actual: describe(s),
            expected: Description::new_base_type("array"),
        }),
    }?;

    // Retrieve the connection pool
    let (connection_id, connection) = {
        context
            .global
            .registry
            .libsql
            .get(&name)
            .ok_or_else(LibSql::missing)
            .map(|r| (r.value().0, r.value().1.clone()))?
    };

    context
        .note_named_resource_used(connection_id, LibSql::static_type(), name)
        .await;

    let mut rows = connection.query(&statement, params_from_iter(p)).await?;

    let mut results = Vec::new();
    while let Some(row) = rows.next().await? {
        let mut result: HashMap<String, StorageValue> = HashMap::new();
        for column in 0..row.column_count() {
            let value = row.get_value(column)?;
            let value = parse_libsql_value(value);
            let column_name = row.column_name(column);
            match column_name {
                Some(c) => {
                    result.insert(c.to_string(), value);
                }
                None => {
                    warn!("Could not find column name for index {}", column);
                }
            }
        }
        results.push(StorageValue::Object(result));
    }

    store_value(store_as, context, step_id, StorageValue::Array(results)).await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn query_one<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let name = eval_opt_string_param("name", parameters, context)?.unwrap_or("default".to_owned());
    let statement = eval_string_param("statement", parameters, context)?;

    let params_param = eval_optional_param_with_default(
        "params",
        parameters,
        context,
        StorageValue::Array(vec![]),
    )?;
    let p: Vec<LibSqlValue> = match params_param {
        StorageValue::Array(v) => Ok(v.into_iter().map(LibSqlValue).collect()),
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: "params".to_owned(),
            actual: describe(s),
            expected: Description::new_base_type("array"),
        }),
    }?;

    // Retrieve the connection pool
    let (connection_id, connection) = {
        context
            .global
            .registry
            .libsql
            .get(&name)
            .ok_or_else(LibSql::missing)
            .map(|r| (r.value().0, r.value().1.clone()))?
    };

    context
        .note_named_resource_used(connection_id, LibSql::static_type(), name)
        .await;

    let mut rows = connection.query(&statement, params_from_iter(p)).await?;

    let mut row_to_return: Option<StorageValue> = None;
    while let Some(row) = rows.next().await? {
        let mut result: HashMap<String, StorageValue> = HashMap::new();
        for column in 0..row.column_count() {
            let value = row.get_value(column)?;
            let value = parse_libsql_value(value);
            let column_name = row.column_name(column);
            match column_name {
                Some(c) => {
                    result.insert(c.to_string(), value);
                }
                None => {
                    warn!("Could not find column name for index {}", column);
                }
            }
        }

        match row_to_return {
            Some(_) => {
                return Err(ExecutionError::LibSqlError {
                    message: "Unexpected number of rows returned, expected one got multiple"
                        .to_owned(),
                    stage: "parse".to_owned(),
                });
            }
            None => row_to_return = Some(StorageValue::Object(result)),
        }
    }

    if let Some(row_to_return) = row_to_return {
        store_value(store_as, context, step_id, row_to_return).await?;
    } else {
        return Err(ExecutionError::LibSqlError {
            message: "Unexpected number of rows returned, expected one got none".to_owned(),
            stage: "parse".to_owned(),
        });
    }

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn execute<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let name = eval_opt_string_param("name", parameters, context)?.unwrap_or("default".to_owned());
    let statement = eval_string_param("statement", parameters, context)?;

    let params_param = eval_optional_param_with_default(
        "params",
        parameters,
        context,
        StorageValue::Array(vec![]),
    )?;
    let p: Vec<LibSqlValue> = match params_param {
        StorageValue::Array(v) => Ok(v.into_iter().map(LibSqlValue).collect()),
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: "params".to_owned(),
            actual: describe(s),
            expected: Description::new_base_type("array"),
        }),
    }?;

    // Retrieve the connection pool
    let (connection_id, connection) = {
        context
            .global
            .registry
            .libsql
            .get(&name)
            .ok_or_else(LibSql::missing)
            .map(|r| (r.value().0, r.value().1.clone()))?
    };

    context
        .note_named_resource_used(connection_id, LibSql::static_type(), name)
        .await;

    let rows_affected = connection.execute(&statement, params_from_iter(p)).await?;

    store_value(
        store_as,
        context,
        step_id,
        StorageValue::Number(rows_affected as f64),
    )
    .await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn drop_connection<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let name = eval_opt_string_param("name", parameters, context)?.unwrap_or("default".to_owned());

    let (name, connection) = context
        .global
        .registry
        .libsql
        .remove(&name)
        .ok_or_else(|| ExecutionError::LibSqlError {
            message: "LibSql connection not found".to_owned(),
            stage: "drop".to_owned(),
        })?;

    context
        .note_named_resource_consumed(connection.0, LibSql::static_type(), name)
        .await;

    Ok(())
}

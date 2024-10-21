use libsql::params::IntoValue;
use libsql::{params_from_iter, Error};
use std::collections::HashMap;
use tel::{describe, Description, StorageValue, NULL};
use tracing::{debug, instrument, warn};
use url::Url;

use crate::{
    actions::as_string,
    errors::ExecutionError,
    evaluations::{eval_optional_param_with_default, eval_param},
    executor::{ExecutionContext, Parameter},
    resources::{LibSql, Resource},
};

use super::store_value;

impl From<Error> for ExecutionError {
    fn from(error: Error) -> Self {
        ExecutionError::SqliteError {
            message: error.to_string(),
            stage: "unknown".into(),
        }
    }
}

pub struct LibSqlValue(StorageValue);

// #[derive(Serialize, Deserialize)]
type LibSqlValueRow = HashMap<String, StorageValue>;

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
    let connection_string_param = eval_param(
        "connectionString",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    let connection_string = as_string(connection_string_param, "connectionString")?;

    // Get name
    let name_param = eval_optional_param_with_default(
        "name",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::String("default".to_owned()),
    )?;
    let name = as_string(name_param, "name")?;

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
                .map_err(|e| ExecutionError::ParameterInvalid {
                    name: "connectionString".into(),
                    message: "Could not parse file path".into(),
                })?;

            debug!("Opening local file: {}", file_path.display());

            libsql::Builder::new_local(file_path)
                .build()
                .await
                .map_err(|e| ExecutionError::SqliteError {
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

    let connection = db.connect().map_err(|e| ExecutionError::SqliteError {
        message: e.to_string(),
        stage: "connection_creation".into(),
    })?;

    // Put to the registry
    context
        .global
        .registry
        .libsql
        .insert(name, LibSql(connection));

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn query<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    // First parse parameters before we do any async stuff
    let connection_name_param = eval_optional_param_with_default(
        "name",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::String("default".to_owned()),
    )?;
    let connection_name = as_string(connection_name_param, "name")?;

    let statement_param = eval_param(
        "statement",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    let statement = as_string(statement_param, "statement")?;

    let params_param = eval_optional_param_with_default(
        "params",
        parameters,
        &context.storage,
        &context.environment,
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
    let connection = {
        context
            .global
            .registry
            .libsql
            .get(&connection_name)
            .ok_or_else(LibSql::missing)
            .map(|r| r.value().0.clone())?
    };

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

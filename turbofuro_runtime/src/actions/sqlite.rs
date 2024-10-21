use deadpool_sqlite::{Config, Runtime};
use rusqlite::types::FromSql;
use rusqlite::types::ToSqlOutput;
use rusqlite::Error;
use rusqlite::ToSql;
use std::collections::HashMap;
use tel::{describe, Description, StorageValue};
use tracing::{debug, instrument, warn};

use crate::resources::SqlitePool;
use crate::{
    actions::as_string,
    errors::ExecutionError,
    evaluations::{eval_optional_param_with_default, eval_param},
    executor::{ExecutionContext, Parameter},
    resources::Resource,
};

use super::store_value;

impl From<Error> for ExecutionError {
    fn from(error: Error) -> Self {
        ExecutionError::SqliteError {
            message: error.to_string(),
        }
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn open_database<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let path_param = eval_param("path", parameters, &context.storage, &context.environment)?;
    let path = as_string(path_param, "path")?;

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
    let exists = { context.global.registry.sqlite_pools.contains_key(&name) };
    if exists {
        return Ok(());
    }

    let cfg = Config::new(path);
    let pool = cfg
        .create_pool(Runtime::Tokio1)
        .map_err(|e| ExecutionError::PostgresError {
            message: e.to_string(),
            stage: "pool_creation".into(),
        })?;

    debug!("Created Sqlite connection pool: {}", name);

    // Put to the registry
    context
        .global
        .registry
        .sqlite_pools
        .insert(name, SqlitePool(pool));

    Ok(())
}

#[derive(Debug, Clone)]
struct QueryArgument(StorageValue);

impl ToSql for QueryArgument {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        match &self.0 {
            StorageValue::String(s) => s.to_sql(),
            StorageValue::Number(f) => f.to_sql(),
            StorageValue::Boolean(b) => b.to_sql(),
            StorageValue::Array(arr) => {
                let json = serde_json::to_string(arr).unwrap();
                Ok(ToSqlOutput::from(json))
            }
            StorageValue::Object(obj) => {
                let json = serde_json::to_string(obj).unwrap();
                Ok(ToSqlOutput::from(json))
            }
            StorageValue::Null(_) => Ok(rusqlite::types::Null.into()),
        }
    }
}

impl FromSql for QueryArgument {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        match value {
            rusqlite::types::ValueRef::Text(s) => Ok(QueryArgument(
                String::from_utf8_lossy(s).into_owned().into(),
            )),
            rusqlite::types::ValueRef::Integer(i) => {
                Ok(QueryArgument(StorageValue::Number(i as f64)))
            }
            rusqlite::types::ValueRef::Real(f) => Ok(QueryArgument(StorageValue::Number(f))),
            rusqlite::types::ValueRef::Null => Ok(QueryArgument(StorageValue::Null(None))),
            _ => Err(rusqlite::types::FromSqlError::InvalidType),
            // TODO: Handle blob
        }
    }
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
    let params: Vec<QueryArgument> = match params_param {
        StorageValue::Array(v) => Ok(v.into_iter().map(QueryArgument).collect()),
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: "params".to_owned(),
            actual: describe(s),
            expected: Description::new_base_type("array"),
        }),
    }?;

    // Retrieve the connection pool
    let sqlite_pool = {
        context
            .global
            .registry
            .sqlite_pools
            .get(&connection_name)
            .ok_or_else(SqlitePool::missing)
            .map(|r| r.value().0.clone())?
    };

    let value = sqlite_pool
        .get()
        .await
        .map_err(|e| ExecutionError::SqliteError {
            message: e.to_string(),
        })?
        .interact(move |conn| {
            let mut statement = conn.prepare_cached(&statement)?;
            let params = rusqlite::params_from_iter(params);
            let column_count = statement.column_count();
            let column_names: Vec<String> = statement
                .column_names()
                .iter()
                .map(|c| c.to_string())
                .collect();
            let row: Result<StorageValue, Error> = statement.query_row(params, |f| {
                let mut result: HashMap<String, StorageValue> = HashMap::new();
                for i in 0..column_count {
                    let value: QueryArgument = f.get(i)?;
                    match column_names.get(i) {
                        Some(c) => {
                            result.insert(c.to_string(), value.0);
                        }
                        None => {
                            warn!("Could not find column name for index {}", i);
                        }
                    }
                }
                Ok(StorageValue::Object(result))
            });
            row
        })
        .await
        .map_err(|e| {
            ExecutionError::SqliteError {
                message: e.to_string(),
                // stage: "query".into(),
            }
        })??;

    store_value(store_as, context, step_id, value).await?;
    Ok(())
}

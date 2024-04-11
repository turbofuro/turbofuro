use chrono::{DateTime, Utc};
use deadpool_postgres::{Config, Runtime};
use std::{collections::HashMap, time::SystemTime};
use tel::{describe, Description, StorageValue};
use tokio_postgres::{
    types::{IsNull, ToSql, Type},
    NoTls, Row,
};
use tracing::{debug, error, instrument, warn};
use url::Url;
use uuid::Uuid;

use crate::{
    actions::as_string,
    errors::ExecutionError,
    evaluations::{eval_optional_param_with_default, eval_param},
    executor::{ExecutionContext, Parameter},
    resources::{PostgresPool, Resource},
};

use super::store_value;

fn postgres_resource_not_found() -> ExecutionError {
    ExecutionError::MissingResource {
        resource_type: PostgresPool::get_type().into(),
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn get_connection<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    // Get connection string
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
    let exists = { context.global.registry.postgres_pools.contains_key(&name) };
    if exists {
        return Ok(());
    }

    // TODO: Add optional TLS support
    // let tls = MakeTlsConnector::new(TlsConnector::new().unwrap());
    let mut cfg = Config::new();
    let url = Url::parse(&connection_string).map_err(|e| ExecutionError::ParameterInvalid {
        name: "connectionString".into(),
        message: e.to_string(),
    })?;
    cfg.user = Some(url.username().to_owned());
    cfg.password = url.password().map(|s| s.to_string());
    cfg.host = url.host_str().map(|s| s.to_string());
    cfg.application_name = Some("Turbofuro".to_owned());
    cfg.port = url.port();
    cfg.dbname = Some(url.path().to_owned().replacen('/', "", 1));

    let pool = cfg.create_pool(Some(Runtime::Tokio1), NoTls).map_err(|e| {
        ExecutionError::PostgresError {
            message: e.to_string(),
            stage: "pool_creation".into(),
        }
    })?;

    debug!("Created Postgres connection pool: {}", name);

    // Put to the registry
    context
        .global
        .registry
        .postgres_pools
        .insert(name, PostgresPool(pool));

    Ok(())
}

fn parse_row(row: Row) -> Result<StorageValue, ExecutionError> {
    let mut result: HashMap<String, StorageValue> = HashMap::new();
    for (i, c) in row.columns().iter().enumerate() {
        let t = c.type_();
        match *t {
            Type::INT4 => {
                let v: i32 = row.get(i);
                result.insert(c.name().to_owned(), StorageValue::Number(v.into()));
            }
            Type::INT8 => {
                let v: i64 = row.get(i);
                result.insert(c.name().to_owned(), StorageValue::Number(v as f64));
            }
            Type::FLOAT4 => {
                let v: f64 = row.get(i);
                result.insert(c.name().to_owned(), StorageValue::Number(v));
            }
            Type::FLOAT8 => {
                let v: f64 = row.get(i);
                result.insert(c.name().to_owned(), StorageValue::Number(v));
            }
            Type::TEXT => {
                let v: String = row.get(i);
                result.insert(c.name().to_owned(), StorageValue::String(v));
            }
            Type::UUID => {
                let v: Uuid = row.get(i);
                result.insert(c.name().to_owned(), StorageValue::String(v.to_string()));
            }
            Type::BOOL => {
                let v: bool = row.get(i);
                result.insert(c.name().to_owned(), StorageValue::Boolean(v));
            }
            Type::TIMESTAMPTZ => {
                let v: SystemTime = row.get(i);
                result.insert(
                    c.name().to_owned(),
                    StorageValue::Number(
                        v.duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as f64,
                    ),
                );
            }
            Type::JSONB => {
                let v: serde_json::Value = row.get(i);
                result.insert(
                    c.name().to_owned(),
                    serde_json::from_value(v).map_err(|_e| ExecutionError::PostgresError {
                        message: "Could not parse JSONB value".to_owned(),
                        stage: "parse".into(),
                    })?,
                );
            }
            Type::JSON => {
                let v: serde_json::Value = row.get(i);
                result.insert(
                    c.name().to_owned(),
                    serde_json::from_value(v).map_err(|_e| ExecutionError::PostgresError {
                        message: "Could not parse JSONB value".to_owned(),
                        stage: "parse".into(),
                    })?,
                );
            }
            _ => {
                warn!("Unsupported type {:?}", t)
            }
        }
    }
    Ok(StorageValue::Object(result))
}

#[derive(Debug, Clone)]
struct QueryArgument(StorageValue);

impl ToSql for QueryArgument {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        match &self.0 {
            StorageValue::String(s) => s.to_sql(ty, out),
            StorageValue::Number(f) => f.to_sql(ty, out),
            StorageValue::Boolean(b) => b.to_sql(ty, out),
            StorageValue::Array(a) => serde_json::to_value(a).unwrap().to_sql(ty, out),
            StorageValue::Object(obj) => serde_json::to_value(obj).unwrap().to_sql(ty, out),
            StorageValue::Null(_) => Ok(IsNull::Yes),
        }
    }

    fn accepts(ty: &Type) -> bool
    where
        Self: Sized,
    {
        String::accepts(ty) || f32::accepts(ty) || i32::accepts(ty) || bool::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match &self.0 {
            StorageValue::String(s) => match ty {
                &Type::TEXT => s.to_sql_checked(&Type::TEXT, out),
                &Type::VARCHAR => s.to_sql_checked(&Type::VARCHAR, out),
                &Type::TIMESTAMPTZ => {
                    let dt = chrono::DateTime::parse_from_rfc3339(s)
                        .map_err(|e| e.to_string())?
                        .with_timezone(&chrono::Utc);
                    SystemTime::from(dt).to_sql_checked(&Type::TIMESTAMPTZ, out)
                }
                &Type::UUID => Uuid::parse_str(s)
                    .map_err(|e| e.to_string())?
                    .to_sql_checked(&Type::UUID, out),
                t => s.to_sql_checked(t, out),
            },
            StorageValue::Number(f) => match ty {
                &Type::INT4 => f.to_sql_checked(&Type::INT4, out),
                &Type::INT8 => f.to_sql_checked(&Type::INT8, out),
                &Type::FLOAT4 => f.to_sql_checked(&Type::FLOAT4, out),
                &Type::FLOAT8 => f.to_sql_checked(&Type::FLOAT8, out),
                &Type::TIMESTAMPTZ => {
                    match chrono::NaiveDateTime::from_timestamp_millis(*f as i64) {
                        Some(dt) => {
                            let dt: DateTime<Utc> = chrono::DateTime::from_utc(dt, chrono::Utc);
                            SystemTime::from(dt).to_sql_checked(&Type::TIMESTAMPTZ, out)
                        }
                        None => Err("Could not convert timestamp".into()),
                    }
                }
                t => f.to_sql_checked(t, out),
            },
            StorageValue::Boolean(b) => b.to_sql_checked(ty, out),
            // TODO: Check that the field is a JSON type
            StorageValue::Array(a) => serde_json::to_value(a).unwrap().to_sql(ty, out),
            StorageValue::Object(obj) => serde_json::to_value(obj).unwrap().to_sql(ty, out),
            StorageValue::Null(_) => Ok(IsNull::Yes),
        }
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn query_one<'a>(
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

    // It is a little bit quirky to get the query params done here
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

    let query_params: Vec<&(dyn ToSql + Sync)> = params
        .iter()
        .map(|x| x as &(dyn ToSql + Sync))
        .collect::<Vec<_>>();

    // Retrieve the connection pool
    let postgres_pool = {
        context
            .global
            .registry
            .postgres_pools
            .get(&connection_name)
            .ok_or_else(postgres_resource_not_found)
            .map(|r| r.value().0.clone())?
    };

    let client = postgres_pool
        .get()
        .await
        .map_err(|e| ExecutionError::PostgresError {
            message: e.to_string(),
            stage: "pool".into(),
        })?;

    let value = parse_row(
        client
            .query_one(&statement, &query_params)
            .await
            .map_err(|e| ExecutionError::PostgresError {
                message: e.to_string(),
                stage: "query".into(),
            })?,
    )?;

    store_value(store_as, context, step_id, value).await?;
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

    // It is a little bit quirky to get the query params done here
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

    let query_params: Vec<&(dyn ToSql + Sync)> = params
        .iter()
        .map(|x| x as &(dyn ToSql + Sync))
        .collect::<Vec<_>>();

    // Retrieve the connection pool
    let postgres_pool = {
        context
            .global
            .registry
            .postgres_pools
            .get(&connection_name)
            .ok_or_else(postgres_resource_not_found)
            .map(|r| r.value().0.clone())?
    };

    let client = postgres_pool
        .get()
        .await
        .map_err(|e| ExecutionError::PostgresError {
            message: e.to_string(),
            stage: "pool".into(),
        })?;

    let value = {
        let mut values: Vec<StorageValue> = vec![];
        for row in client.query(&statement, &query_params).await.map_err(|e| {
            ExecutionError::PostgresError {
                message: e.to_string(),
                stage: "query".into(),
            }
        })? {
            values.push(parse_row(row)?);
        }
        StorageValue::Array(values)
    };

    store_value(store_as, context, step_id, value).await?;
    Ok(())
}

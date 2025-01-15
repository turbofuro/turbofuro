use deadpool_postgres::{Config, Runtime};
use std::{collections::HashMap, time::SystemTime};
use tel::{describe, Description, StorageValue, NULL};
use tokio_postgres::{
    types::{IsNull, ToSql, Type},
    NoTls, Row,
};
use tracing::{debug, instrument, warn};
use url::Url;
use uuid::Uuid;

use crate::{
    errors::ExecutionError,
    evaluations::{eval_opt_string_param, eval_optional_param_with_default, eval_string_param},
    executor::{ExecutionContext, Parameter},
    resources::{generate_resource_id, PostgresPool, Resource},
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

    // Test the connection
    let conn = pool
        .get()
        .await
        .map_err(|e| ExecutionError::PostgresError {
            message: e.to_string(),
            stage: "pool_test".into(),
        })?;
    conn.query("SELECT 1", &[])
        .await
        .map_err(|e| ExecutionError::PostgresError {
            message: e.to_string(),
            stage: "query".into(),
        })?;

    debug!("Created Postgres connection pool: {}", name);

    // Put to the registry
    let pool_id = generate_resource_id();
    context
        .global
        .registry
        .postgres_pools
        .insert(name.clone(), PostgresPool(pool_id, pool));
    context
        .note_named_resource_provisioned(pool_id, PostgresPool::static_type(), name)
        .await;

    Ok(())
}

fn parse_row(row: Row) -> Result<StorageValue, ExecutionError> {
    let mut result: HashMap<String, StorageValue> = HashMap::new();
    for (i, c) in row.columns().iter().enumerate() {
        let t = c.type_();
        match *t {
            Type::INT4 => {
                let v = row.try_get::<usize, Option<i32>>(i).map_err(|e| {
                    ExecutionError::PostgresError {
                        message: e.to_string(),
                        stage: "parse".into(),
                    }
                })?;
                result.insert(c.name().to_owned(), v.map_or(NULL, |v| v.into()));
            }
            Type::INT8 => {
                let v = row.try_get::<usize, Option<i64>>(i).map_err(|e| {
                    ExecutionError::PostgresError {
                        message: e.to_string(),
                        stage: "parse".into(),
                    }
                })?;
                result.insert(c.name().to_owned(), v.map_or(NULL, |v| (v as f64).into()));
            }
            Type::FLOAT4 => {
                let v = row.try_get::<usize, Option<f64>>(i).map_err(|e| {
                    ExecutionError::PostgresError {
                        message: e.to_string(),
                        stage: "parse".into(),
                    }
                })?;
                result.insert(c.name().to_owned(), v.map_or(NULL, StorageValue::Number));
            }
            Type::FLOAT8 => {
                let v = row.try_get::<usize, Option<f64>>(i).map_err(|e| {
                    ExecutionError::PostgresError {
                        message: e.to_string(),
                        stage: "parse".into(),
                    }
                })?;
                result.insert(c.name().to_owned(), v.map_or(NULL, StorageValue::Number));
            }
            Type::TEXT => {
                let v = row.try_get::<usize, Option<String>>(i).map_err(|e| {
                    ExecutionError::PostgresError {
                        message: e.to_string(),
                        stage: "parse".into(),
                    }
                })?;
                result.insert(c.name().to_owned(), v.map_or(NULL, StorageValue::String));
            }
            Type::UUID => {
                let v = row.try_get::<usize, Option<Uuid>>(i).map_err(|e| {
                    ExecutionError::PostgresError {
                        message: e.to_string(),
                        stage: "parse".into(),
                    }
                })?;
                result.insert(
                    c.name().to_owned(),
                    v.map_or(NULL, |v| v.to_string().into()),
                );
            }
            Type::BOOL => {
                let v = row.try_get::<usize, Option<bool>>(i).map_err(|e| {
                    ExecutionError::PostgresError {
                        message: e.to_string(),
                        stage: "parse".into(),
                    }
                })?;
                result.insert(c.name().to_owned(), v.map_or(NULL, |v| v.into()));
            }
            Type::TIMESTAMPTZ => {
                let v: Option<SystemTime> =
                    row.try_get(i).map_err(|e| ExecutionError::PostgresError {
                        message: e.to_string(),
                        stage: "parse".into(),
                    })?;
                result.insert(
                    c.name().to_owned(),
                    v.map_or(NULL, |v| {
                        StorageValue::Number(
                            v.duration_since(SystemTime::UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as f64,
                        )
                    }),
                );
            }
            Type::JSONB => {
                let v = row
                    .try_get::<usize, Option<serde_json::Value>>(i)
                    .map_err(|e| ExecutionError::PostgresError {
                        message: e.to_string(),
                        stage: "parse".into(),
                    })?;

                match v {
                    Some(v) => {
                        result.insert(
                            c.name().to_owned(),
                            serde_json::from_value(v).map_err(|_e| {
                                ExecutionError::PostgresError {
                                    message: "Could not parse JSONB value".to_owned(),
                                    stage: "parse".into(),
                                }
                            })?,
                        );
                    }
                    None => {
                        result.insert(c.name().to_owned(), NULL);
                    }
                }
            }
            Type::JSON => {
                let v = row
                    .try_get::<usize, Option<serde_json::Value>>(i)
                    .map_err(|e| ExecutionError::PostgresError {
                        message: e.to_string(),
                        stage: "parse".into(),
                    })?;

                match v {
                    Some(v) => {
                        result.insert(
                            c.name().to_owned(),
                            serde_json::from_value(v).map_err(|_e| {
                                ExecutionError::PostgresError {
                                    message: "Could not parse JSONB value".to_owned(),
                                    stage: "parse".into(),
                                }
                            })?,
                        );
                    }
                    None => {
                        result.insert(c.name().to_owned(), NULL);
                    }
                }
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
        String::accepts(ty) || f64::accepts(ty) || i32::accepts(ty) || bool::accepts(ty)
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
                &Type::INT4 => (*f as i32).to_sql_checked(&Type::INT4, out),
                &Type::INT8 => (*f as i64).to_sql_checked(&Type::INT8, out),
                &Type::FLOAT4 => f.to_sql_checked(&Type::FLOAT4, out),
                &Type::FLOAT8 => f.to_sql_checked(&Type::FLOAT8, out),
                &Type::TIMESTAMPTZ => match chrono::DateTime::from_timestamp_millis(*f as i64) {
                    Some(dt) => SystemTime::from(dt).to_sql_checked(&Type::TIMESTAMPTZ, out),
                    None => Err("Could not convert timestamp".into()),
                },
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
    let name = eval_opt_string_param("name", parameters, context)?.unwrap_or("default".to_owned());
    let statement = eval_string_param("statement", parameters, context)?;

    // It is a little bit quirky to get the query params done here
    let params_param = eval_optional_param_with_default(
        "params",
        parameters,
        context,
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
    let (pool_id, postgres_pool) = {
        context
            .global
            .registry
            .postgres_pools
            .get(&name)
            .ok_or_else(PostgresPool::missing)
            .map(|r| (r.value().0, r.value().1.clone()))?
    };

    context
        .note_named_resource_used(pool_id, PostgresPool::static_type(), name)
        .await;

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
    let name = eval_opt_string_param("name", parameters, context)?.unwrap_or("default".to_owned());
    let statement = eval_string_param("statement", parameters, context)?;

    // It is a little bit quirky to get the query params done here
    let params_param = eval_optional_param_with_default(
        "params",
        parameters,
        context,
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
    let (pool_id, postgres_pool) = {
        context
            .global
            .registry
            .postgres_pools
            .get(&name)
            .ok_or_else(PostgresPool::missing)
            .map(|r| (r.value().0, r.value().1.clone()))?
    };

    context
        .note_named_resource_used(pool_id, PostgresPool::static_type(), name)
        .await;

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

#[instrument(level = "trace", skip_all)]
pub async fn execute<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let name = eval_opt_string_param("name", parameters, context)?.unwrap_or("default".to_owned());
    let statement = eval_string_param("statement", parameters, context)?;

    // It is a little bit quirky to get the query params done here
    let params_param = eval_optional_param_with_default(
        "params",
        parameters,
        context,
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
    let (pool_id, postgres_pool) = {
        context
            .global
            .registry
            .postgres_pools
            .get(&name)
            .ok_or_else(PostgresPool::missing)
            .map(|r| (r.value().0, r.value().1.clone()))?
    };

    context
        .note_named_resource_used(pool_id, PostgresPool::static_type(), name)
        .await;

    let client = postgres_pool
        .get()
        .await
        .map_err(|e| ExecutionError::PostgresError {
            message: e.to_string(),
            stage: "pool".into(),
        })?;

    let rows_affected = client
        .execute(&statement, &query_params)
        .await
        .map_err(|e| ExecutionError::PostgresError {
            message: e.to_string(),
            stage: "query".into(),
        })?;

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

    let (name, pool) = context
        .global
        .registry
        .postgres_pools
        .remove(&name)
        .ok_or_else(|| ExecutionError::PostgresError {
            message: "Postgres pool not found".to_owned(),
            stage: "drop".to_owned(),
        })?;
    context
        .note_named_resource_consumed(pool.0, PostgresPool::static_type(), name)
        .await;

    Ok(())
}

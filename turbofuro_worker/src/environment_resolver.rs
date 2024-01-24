use async_trait::async_trait;
use futures_util::TryFutureExt;
use moka::future::Cache;
use reqwest::Client;
use std::sync::Arc;
use tokio::{fs::File, io::AsyncReadExt, sync::Mutex};
use tracing::error;
use turbofuro_runtime::executor::Environment;

use crate::worker::AppError;

#[async_trait]
pub trait EnvironmentResolver: Send + Sync {
    async fn get_environment(&mut self, id: &str) -> Result<Environment, AppError>;
}

pub type SharedEnvironmentResolver = Arc<Mutex<dyn EnvironmentResolver>>;

pub struct FileSystemEnvironmentResolver {}

#[async_trait]
impl EnvironmentResolver for FileSystemEnvironmentResolver {
    async fn get_environment(&mut self, id: &str) -> Result<Environment, AppError> {
        let path = format!("test_environments/{}.json", id);
        let mut file = File::open(path)
            .map_err(|_| AppError::EnvironmentNotFound)
            .await?;

        let mut buffer = String::new();
        file.read_to_string(&mut buffer)
            .map_err(|_| AppError::MalformedEnvironment)
            .await?;

        let environment: Environment = serde_json::from_str(&buffer).map_err(|_| {
            error!("Failed to parse environment: {}", buffer);
            AppError::MalformedEnvironment
        })?;

        Ok(environment)
    }
}

pub struct ManagerStorageEnvironmentResolver {
    pub client: Client,
    pub base_url: String,

    token: String,
    cache: Cache<String, Environment>,
}

impl ManagerStorageEnvironmentResolver {
    pub fn new(client: Client, base_url: String, token: String) -> Self {
        let cache = Cache::<String, Environment>::new(4);

        Self {
            client,
            base_url,
            cache,
            token,
        }
    }
}

#[async_trait]
impl EnvironmentResolver for ManagerStorageEnvironmentResolver {
    async fn get_environment(&mut self, id: &str) -> Result<Environment, AppError> {
        if let Some(cached) = self.cache.get(id).await {
            return Ok(cached);
        }

        let environment: Environment = self
            .client
            .get(format!(
                "{}/mission-control/environment/{}",
                self.base_url, id
            ))
            .header("x-turbofuro-token", &self.token)
            .send()
            .await
            // .map(|res| {
            //     print!("{:?}", res);
            //     res
            // })
            .map_err(|err| {
                error!("Failed to get environment: {}", err);
                AppError::EnvironmentNotFound
            })?
            .json()
            .await
            .map_err(|err| {
                error!("Malformed environment {}", err);
                AppError::MalformedEnvironment
            })?;

        self.cache.insert(id.to_string(), environment.clone()).await;

        Ok(environment)
    }
}

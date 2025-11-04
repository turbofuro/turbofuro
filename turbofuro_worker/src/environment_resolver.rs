use async_trait::async_trait;
use futures_util::TryFutureExt;
use reqwest::Client;
use std::sync::Arc;
use tokio::{fs::File, io::AsyncReadExt, sync::Mutex};
use tracing::error;
use turbofuro_runtime::executor::Environment;

use crate::errors::WorkerError;

#[async_trait]
pub trait EnvironmentResolver: Send + Sync {
    async fn get_environment(&mut self, id: &str) -> Result<Environment, WorkerError>;
}

pub type SharedEnvironmentResolver = Arc<Mutex<dyn EnvironmentResolver>>;

pub struct FileSystemEnvironmentResolver {}

#[async_trait]
impl EnvironmentResolver for FileSystemEnvironmentResolver {
    async fn get_environment(&mut self, id: &str) -> Result<Environment, WorkerError> {
        let path = format!("test_environments/{id}.json");
        let mut file = File::open(path)
            .map_err(|_| WorkerError::EnvironmentNotFound)
            .await?;

        let mut buffer = String::new();
        file.read_to_string(&mut buffer)
            .map_err(|_| WorkerError::MalformedEnvironment)
            .await?;

        let environment: Environment = serde_json::from_str(&buffer).map_err(|_| {
            error!("Failed to parse environment: {}", buffer);
            WorkerError::MalformedEnvironment
        })?;

        Ok(environment)
    }
}

pub struct CloudEnvironmentResolver {
    pub client: Client,
    pub base_url: String,

    token: String,
}

impl CloudEnvironmentResolver {
    pub fn new(client: Client, base_url: String, token: String) -> Self {
        Self {
            client,
            base_url,
            token,
        }
    }
}

#[async_trait]
impl EnvironmentResolver for CloudEnvironmentResolver {
    async fn get_environment(&mut self, id: &str) -> Result<Environment, WorkerError> {
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
                WorkerError::EnvironmentNotFound
            })?
            .json()
            .await
            .map_err(|err| {
                error!("Malformed environment {}", err);
                WorkerError::MalformedEnvironment
            })?;

        Ok(environment)
    }
}

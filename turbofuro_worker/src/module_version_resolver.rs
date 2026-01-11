use std::sync::Arc;

use crate::{errors::WorkerError, shared::ModuleVersion};
use async_trait::async_trait;
use futures_util::TryFutureExt;
use http::StatusCode;
use moka::future::Cache;
use reqwest::Client;
use tokio::{fs::File, io::AsyncReadExt};
use tracing::{error, instrument};

#[async_trait]
pub trait ModuleVersionResolver: Send + Sync {
    async fn get_module_version(&self, id: &str) -> Result<ModuleVersion, WorkerError>;
}

pub type SharedModuleVersionResolver = Arc<dyn ModuleVersionResolver>;

#[derive(Debug)]
pub struct FileSystemModuleVersionResolver {}

#[async_trait]
impl ModuleVersionResolver for FileSystemModuleVersionResolver {
    #[instrument(level = "info", skip(self))]
    async fn get_module_version(&self, id: &str) -> Result<ModuleVersion, WorkerError> {
        let path = format!("test_module_versions/{id}.json");
        let mut file = File::open(path)
            .map_err(|_| WorkerError::ModuleVersionNotFound)
            .await?;

        let mut buffer = String::new();
        file.read_to_string(&mut buffer)
            .map_err(|_| WorkerError::MalformedModuleVersion)
            .await?;

        let module: ModuleVersion = serde_json::from_str(&buffer).map_err(|_| {
            error!("Failed to parse module version: {}", buffer);
            WorkerError::MalformedModuleVersion
        })?;

        Ok(module)
    }
}

#[derive(Debug)]
pub struct CloudModuleVersionResolver {
    pub client: Client,
    pub base_url: String,

    token: String,
    cache: Cache<String, ModuleVersion>,
}

impl CloudModuleVersionResolver {
    pub fn new(client: Client, base_url: String, token: String) -> Self {
        let cache = Cache::<String, ModuleVersion>::new(128);

        Self {
            client,
            base_url,
            cache,
            token,
        }
    }
}

#[async_trait]
impl ModuleVersionResolver for CloudModuleVersionResolver {
    #[instrument(level = "info", skip(self))]
    async fn get_module_version(&self, id: &str) -> Result<ModuleVersion, WorkerError> {
        if let Some(cached) = self.cache.get(id).await {
            return Ok(cached);
        }

        let response = self
            .client
            .get(format!("{}/mission-control/module/{}", self.base_url, id))
            .header("x-turbofuro-token", &self.token)
            .send()
            .await
            .map_err(|err| {
                error!(
                    "Failed to get module version id: {}, error was: {}",
                    id, err
                );
                WorkerError::ModuleRepositoryUnavailable
            })?;

        if response.status() == StatusCode::NOT_FOUND {
            return Err(WorkerError::ModuleVersionNotFound);
        } else if !response.status().is_success() {
            return Err(WorkerError::ModuleRepositoryUnavailable);
        }

        let module_version: ModuleVersion = response.json().await.map_err(|err| {
            error!("Malformed module version id: {}, error was: {}", id, err);
            WorkerError::MalformedModuleVersion
        })?;

        self.cache
            .insert(id.to_string(), module_version.clone())
            .await;

        Ok(module_version)
    }
}

use std::{sync::Arc, time::Duration};

use serde_derive::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::{errors::WorkerError, options::CloudOptions};

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Cors {
    Disabled,
    Any,
    Origins { origins: Vec<String> },
    AnyWithCredentials,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Compression {
    Disabled,
    Automatic,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Timeout {
    Disabled,
    Default,
    Custom { seconds: u64 },
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSettings {
    pub cors: Cors,
    pub compression: Compression,
    pub timeout: Timeout,
}

impl Default for WorkerSettings {
    fn default() -> Self {
        WorkerSettings {
            cors: Cors::Disabled,
            compression: Compression::Automatic,
            timeout: Timeout::Default,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Configuration {
    pub id: String,
    pub modules: Vec<ModuleSpec>,
    pub environment_id: Option<String>,
    #[serde(default)]
    pub settings: WorkerSettings,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModuleSpec {
    pub module_version_id: String,
    pub module_id: String,
}

pub type SharedConfiguration = Arc<Mutex<Configuration>>;

pub async fn fetch_configuration(options: &CloudOptions) -> Result<Configuration, WorkerError> {
    let url = format!("{}/mission-control/configuration", options.cloud_url);

    let response = reqwest::Client::new()
        .get(url)
        .header("x-turbofuro-token", options.token.to_owned())
        .header("content-length", 0)
        .send()
        .await
        .map_err(|e| WorkerError::CouldNotFetchConfiguration {
            message: format!("Failed to fetch configuration: {e}"),
        })?;

    if !response.status().is_success() {
        return Err(WorkerError::CouldNotFetchConfiguration {
            message: format!(
                "Failed to fetch configuration. Status code was {}",
                response.status()
            ),
        });
    }

    let response_body =
        response
            .text()
            .await
            .map_err(|e| WorkerError::CouldNotFetchConfiguration {
                message: format!(
                    "Failed to fetch configuration. Could not read response body: {e}"
                ),
            })?;

    let config: Configuration =
        serde_json::from_str(&response_body).map_err(|e| WorkerError::MalformedConfiguration {
            message: format!("Failed to parse configuration: {e}"),
        })?;

    debug!(
        "Configuration fetched:\n{}",
        serde_json::to_string_pretty(&config).unwrap()
    );

    Ok(config)
}

/// A simple passive configuration fetcher that runs in the background
///
/// This is a backup mechanism in case there is an issue with Cloud Agent.
pub fn run_configuration_fetcher(
    cloud_options: CloudOptions,
    configuration_coordinator: ConfigurationCoordinator,
) {
    tokio::spawn(async move {
        loop {
            let result = fetch_configuration(&cloud_options).await;

            match result {
                Ok(new_config) => {
                    configuration_coordinator
                        .update_configuration(new_config)
                        .await;
                }
                Err(e) => {
                    warn!("Passive configuration fetch failed with error: {:?}", e);
                }
            }
            tokio::time::sleep(Duration::from_secs(3000)).await; // 10 minutes
        }
    });
}

#[derive(Debug, Clone)]
pub struct ConfigurationCoordinator(tokio::sync::mpsc::Sender<Configuration>);

impl ConfigurationCoordinator {
    pub async fn update_configuration(&self, config: Configuration) {
        self.0.send(config).await.unwrap();
    }
}

/// Configuration coordinator that receives new configurations and updates the shared configuration if necessary.
/// This is convenient as there are multiple sources of configuration updates.
pub fn run_configuration_coordinator(
    config: SharedConfiguration,
    update_sender: tokio::sync::broadcast::Sender<String>,
) -> ConfigurationCoordinator {
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<Configuration>(16);
    tokio::spawn(async move {
        while let Some(new_config) = receiver.recv().await {
            let mut current_config = config.lock().await;

            if new_config.id == current_config.id {
                debug!(
                    "New configuration received, but it is the same as the current one. Ignoring."
                );
                continue;
            }

            info!("New configuration received");
            *current_config = new_config;
            update_sender.send(current_config.id.clone()).unwrap();
        }
    });
    ConfigurationCoordinator(sender)
}

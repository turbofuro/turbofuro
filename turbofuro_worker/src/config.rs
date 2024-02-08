use std::{sync::Arc, time::Duration};

use serde_derive::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::{worker::WorkerError, CloudOptions};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Configuration {
    pub id: String,
    pub modules: Vec<ModuleSpec>,
    pub environment_id: String,
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
        .map_err(|e| {
            warn!("Failed to fetch configuration {}", e);
            WorkerError::ConfigurationFetch
        })?;

    let response_body = response.text().await.map_err(|e| {
        error!("Failed to read configuration response body {}", e);
        WorkerError::ConfigurationFetch
    })?;
    let config: Configuration = serde_json::from_str(&response_body).map_err(|e| {
        error!(
            "Failed to parse configuration\nError is {}\nResponse body is {}",
            e, response_body
        );
        WorkerError::ConfigurationFetch
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
            let new_config = fetch_configuration(&cloud_options).await.unwrap(); // TODO: Error handling
            {
                configuration_coordinator
                    .update_configuration(new_config)
                    .await
            }
            tokio::time::sleep(Duration::from_secs(3000)).await; // TODO: Increase this
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

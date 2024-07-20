use std::{
    net::{SocketAddr, SocketAddrV4},
    str::FromStr,
};

use reqwest::Url;
use tracing::info;

use crate::{cli::AppArgs, errors::WorkerError};

#[derive(Debug, Clone, PartialEq)]
pub struct HttpServerOptions {
    pub port: u16,
    pub addr: String,
}

impl Default for HttpServerOptions {
    fn default() -> Self {
        Self {
            port: 4000,
            addr: "0.0.0.0".to_owned(),
        }
    }
}

impl HttpServerOptions {
    pub fn get_socket_addr(&self) -> Result<SocketAddr, WorkerError> {
        let socket_addr = format!("{}:{}", self.addr, self.port);

        SocketAddrV4::from_str(&socket_addr)
            .map_err(|e| WorkerError::InvalidArguments {
                message: format!(
                    "Could not parse socket address \"{}\", error was: {:?}",
                    socket_addr, e
                ),
            })
            .map(|socket_addr| socket_addr.into())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CloudOptions {
    pub cloud_url: String,
    pub operator_url: String,
    pub token: String,
    pub name: String,
}

impl CloudOptions {
    pub fn get_operator_url(&self, worker_id: String) -> Result<Url, WorkerError> {
        let query_params =
            serde_urlencoded::to_string([("token", self.token.clone()), ("id", worker_id)])
                .map_err(|_| WorkerError::InvalidOperatorUrl {
                    url: self.operator_url.clone(),
                })?;
        let url_string = format!("{}/server?{}", self.operator_url, query_params,);
        Url::parse(&url_string).map_err(|_| WorkerError::InvalidOperatorUrl {
            url: url_string.clone(),
        })
    }
}

// Remember to update the help message in cli.rs when changing those
static DEFAULT_CLOUD_URL: &str = "https://api.turbofuro.com";
static DEFAULT_OPERATOR_URL: &str = "wss://operator.turbofuro.com";
static TURBOFURO_TOKEN_ENV_NAME: &str = "TURBOFURO_TOKEN";
static TURBOFURO_CLOUD_URL_ENV_NAME: &str = "TURBOFURO_CLOUD_URL";
static TURBOFURO_OPERATOR_URL_ENV_NAME: &str = "TURBOFURO_OPERATOR_URL";
static PORT_ENV_NAME: &str = "PORT";
static NAME_ENV_NAME: &str = "NAME";

pub fn get_cloud_options(args: AppArgs, name: String, token: String) -> CloudOptions {
    let cloud_options = CloudOptions {
        cloud_url: args
            .cloud_url
            .or_else(|| std::env::var(TURBOFURO_CLOUD_URL_ENV_NAME).ok())
            .unwrap_or(DEFAULT_CLOUD_URL.to_owned()),
        operator_url: args
            .operator_url
            .or_else(|| std::env::var(TURBOFURO_OPERATOR_URL_ENV_NAME).ok())
            .unwrap_or(DEFAULT_OPERATOR_URL.to_owned()),
        token,
        name,
    };

    if cloud_options.cloud_url != DEFAULT_CLOUD_URL {
        info!(
            "Using custom cloud URL: {}",
            cloud_options.cloud_url.clone()
        );
    }
    if cloud_options.operator_url != DEFAULT_OPERATOR_URL {
        info!(
            "Using custom operator URL: {}",
            cloud_options.operator_url.clone()
        );
    }
    cloud_options
}

pub fn get_http_server_options(args: AppArgs) -> Result<HttpServerOptions, WorkerError> {
    let addr = match args.address {
        Some(a) => a,
        None => "0.0.0.0".to_owned(),
    };

    let port: u16 = match args.port {
        Some(p) => p,
        None => std::env::var(PORT_ENV_NAME)
            .ok()
            .unwrap_or("4000".to_owned())
            .parse::<u16>()
            .map_err(|e| WorkerError::InvalidEnvironmentVariable {
                name: PORT_ENV_NAME.into(),
                message: e.to_string(),
            })?,
    };

    Ok(HttpServerOptions { addr, port })
}

pub fn get_turbofuro_token(args: AppArgs) -> Option<String> {
    args.token
        .or_else(|| std::env::var(TURBOFURO_TOKEN_ENV_NAME).ok())
}

pub fn get_name(args: AppArgs) -> String {
    args.name
        .or_else(|| std::env::var(NAME_ENV_NAME).ok())
        .or_else(|| {
            {
                hostname::get()
                    .map(|s| s.to_string_lossy().into_owned())
                    .ok()
            }
            .map(|s| s.chars().take(200).collect())
        })
        .unwrap_or("Unknown".to_owned())
}

use axum::response::{IntoResponse, Response};
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use turbofuro_runtime::errors::ExecutionError;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "code", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkerError {
    ModuleVersionNotFound,
    MalformedModuleVersion,
    EnvironmentNotFound,
    MalformedEnvironment,
    InvalidArguments { message: String },
    InvalidEnvironmentVariable { name: String, message: String },
    ExecutionFailed { error: Box<ExecutionError> },
    CouldNotFetchConfiguration { message: String },
    MalformedConfiguration { message: String },
    InvalidCloudAgentCommand { message: String },
    InvalidOperatorUrl { url: String },
    Unsupported { message: String },
}

impl From<pico_args::Error> for WorkerError {
    fn from(error: pico_args::Error) -> Self {
        WorkerError::InvalidArguments {
            message: error.to_string(),
        }
    }
}

impl From<ExecutionError> for WorkerError {
    fn from(error: ExecutionError) -> Self {
        WorkerError::ExecutionFailed {
            error: Box::new(error),
        }
    }
}

// Default error message for worker errors
static DEFAULT_ERROR_MESSAGE: &str = "Turbofuro Worker encountered an error. Check logs or status in the web interface for more information.";

impl IntoResponse for WorkerError {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, DEFAULT_ERROR_MESSAGE).into_response()
    }
}

impl Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkerError::ModuleVersionNotFound => write!(f, "Module version not found"),
            WorkerError::MalformedModuleVersion => write!(f, "Malformed module version"),
            WorkerError::EnvironmentNotFound => write!(f, "Environment not found"),
            WorkerError::MalformedEnvironment => write!(f, "Malformed environment"),
            WorkerError::InvalidArguments { message } => {
                write!(f, "Invalid arguments: {message}")
            }
            WorkerError::InvalidEnvironmentVariable { name, message } => {
                write!(f, "Invalid environment variable {name}: {message}")
            }
            WorkerError::ExecutionFailed { error } => {
                write!(f, "Execution failed: {error}")
            }
            WorkerError::CouldNotFetchConfiguration { message } => {
                write!(f, "Could not fetch configuration: {message}")
            }
            WorkerError::MalformedConfiguration { message } => {
                write!(f, "Malformed configuration: {message}")
            }
            WorkerError::InvalidCloudAgentCommand { message } => {
                write!(f, "Invalid cloud agent command: {message}")
            }
            WorkerError::InvalidOperatorUrl { url } => {
                write!(f, "Invalid operator URL: {url}")
            }
            WorkerError::Unsupported { message } => {
                write!(f, "Unsupported operation: {message}")
            }
        }
    }
}

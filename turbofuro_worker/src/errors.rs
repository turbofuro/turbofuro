use axum::response::{IntoResponse, Response};
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use turbofuro_runtime::errors::ExecutionError;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "app_error", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkerError {
    ModuleVersionNotFound,
    MalformedModuleVersion,
    EnvironmentNotFound,
    MalformedEnvironment,
    InvalidCommandLineArguments {
        message: String,
    },
    InvalidEnvironmentVariable {
        name: String,
        message: String,
    },
    ExecutionFailed {
        #[serde(flatten)]
        error: ExecutionError,
    },
    CouldNotFetchConfiguration {
        message: String,
    },
    MalformedConfiguration {
        message: String,
    },
}

impl From<pico_args::Error> for WorkerError {
    fn from(error: pico_args::Error) -> Self {
        WorkerError::InvalidCommandLineArguments {
            message: error.to_string(),
        }
    }
}

impl From<ExecutionError> for WorkerError {
    fn from(error: ExecutionError) -> Self {
        WorkerError::ExecutionFailed { error }
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
            WorkerError::InvalidCommandLineArguments { message } => {
                write!(f, "Invalid command line arguments: {}", message)
            }
            WorkerError::InvalidEnvironmentVariable { name, message } => {
                write!(f, "Invalid environment variable {}: {}", name, message)
            }
            WorkerError::ExecutionFailed { error } => {
                write!(f, "Execution failed: {}", error)
            }
            WorkerError::CouldNotFetchConfiguration { message } => {
                write!(f, "Could not fetch configuration: {}", message)
            }
            WorkerError::MalformedConfiguration { message } => {
                write!(f, "Malformed configuration: {}", message)
            }
        }
    }
}

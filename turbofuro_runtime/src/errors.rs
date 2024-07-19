use std::{error::Error, fmt::Display};

use redis::RedisError;
use serde::{Deserialize, Serialize};
use tel::{Description, StorageValue, TelError};
use tokio::sync::mpsc::error::SendError;

use crate::actor::ActorCommand;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "code", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionError {
    Custom {
        value: Option<StorageValue>,
    },
    Tel {
        #[serde(flatten)]
        error: TelError,
    },
    MissingParameter {
        name: String,
    },
    ParameterTypeMismatch {
        name: String,
        expected: Description,
        actual: Description,
    },
    ParameterInvalid {
        name: String,
        message: String,
    },
    Unknown {
        message: String,
    },
    StateInvalid {
        message: String,
        subject: String,
        inner: String,
    },
    MissingResource {
        #[serde(rename = "type")]
        resource_type: String,
    },
    Continue,
    Break,
    Return {
        value: StorageValue,
    },
    HandlerNotFound {
        name: String,
    },
    FunctionNotFound {
        id: String,
    },
    ModuleVersionNotFound {
        id: String,
    },
    UnresolvedImport {
        #[serde(rename = "importName")]
        import_name: String,
    },
    Unsupported {
        message: String,
    },
    SerializationFailed {
        message: String,
        breadcrumbs: String,
        inner: String,
    },
    JwtDecodingFailed {
        message: String,
    },
    ActorCommandFailed {
        message: String,
    },
    PostgresError {
        message: String,
        stage: String,
    },
    RedisError {
        message: String,
    },
    IoError {
        message: String,
        // os_code: Option<i32>,
    },
    WasmError {
        message: String,
    },
    WatcherError {
        message: String,
    },
}

impl Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionError::Custom { value } => {
                write!(f, "Custom error: {:?}", value)
            }
            ExecutionError::Tel { error } => {
                write!(f, "Tel error: {}", error)
            }
            ExecutionError::MissingParameter { name } => {
                write!(f, "Missing parameter: {}", name)
            }
            ExecutionError::ParameterTypeMismatch {
                name,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Parameter type mismatch: {} expected: {} actual: {}",
                    name, expected, actual
                )
            }
            ExecutionError::ParameterInvalid { name, message } => {
                write!(f, "Parameter invalid: {} message: {}", name, message)
            }
            ExecutionError::Unknown { message } => {
                write!(f, "Unknown error: {}", message)
            }
            ExecutionError::StateInvalid {
                message,
                subject,
                inner,
            } => {
                write!(
                    f,
                    "State invalid: {} subject: {} inner: {}",
                    message, subject, inner
                )
            }
            ExecutionError::MissingResource { resource_type } => {
                write!(f, "Missing resource: {}", resource_type)
            }
            ExecutionError::Continue => {
                write!(f, "Continue")
            }
            ExecutionError::Break => {
                write!(f, "Break")
            }
            ExecutionError::Return { value } => {
                write!(f, "Return: {:?}", value)
            }
            ExecutionError::HandlerNotFound { name } => {
                write!(f, "Handler not found: {}", name)
            }
            ExecutionError::FunctionNotFound { id } => {
                write!(f, "Function not found: {}", id)
            }
            ExecutionError::ModuleVersionNotFound { id } => {
                write!(f, "Module version not found: {}", id)
            }
            ExecutionError::UnresolvedImport { import_name } => {
                write!(f, "Unresolved import: {}", import_name)
            }
            ExecutionError::Unsupported { message } => {
                write!(f, "Unsupported: {}", message)
            }
            ExecutionError::SerializationFailed {
                message,
                breadcrumbs,
                inner,
            } => {
                write!(
                    f,
                    "Serialization failed: {} breadcrumbs: {} inner: {}",
                    message, breadcrumbs, inner
                )
            }
            ExecutionError::JwtDecodingFailed { message } => {
                write!(f, "JWT decoding failed: {}", message)
            }
            ExecutionError::ActorCommandFailed { message } => {
                write!(f, "Actor command failed: {}", message)
            }
            ExecutionError::PostgresError { message, stage } => {
                write!(f, "Postgres error: {} stage: {}", message, stage)
            }
            ExecutionError::RedisError { message } => {
                write!(f, "Redis error: {}", message)
            }
            ExecutionError::IoError { message } => {
                write!(f, "IO error: {}", message)
            }
            ExecutionError::WasmError { message } => {
                write!(f, "Wasm error: {}", message)
            }
            ExecutionError::WatcherError { message } => {
                write!(f, "Watcher error: {}", message)
            }
        }
    }
}

impl ExecutionError {
    pub fn new_missing_response_from_actor() -> Self {
        ExecutionError::ActorCommandFailed {
            message: "Missing response from actor".to_owned(),
        }
    }
}

impl Error for ExecutionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }

    fn description(&self) -> &str {
        "description() is deprecated; use Display"
    }

    fn cause(&self) -> Option<&dyn Error> {
        self.source()
    }
}

impl From<SendError<ActorCommand>> for ExecutionError {
    fn from(error: SendError<ActorCommand>) -> Self {
        ExecutionError::ActorCommandFailed {
            message: format!("Could not send actor command: {}", error.0),
        }
    }
}

impl From<TelError> for ExecutionError {
    fn from(error: TelError) -> Self {
        ExecutionError::Tel { error }
    }
}

impl From<notify::Error> for ExecutionError {
    fn from(error: notify::Error) -> Self {
        ExecutionError::WatcherError {
            message: error.to_string(),
        }
    }
}

impl From<std::io::Error> for ExecutionError {
    fn from(error: std::io::Error) -> Self {
        ExecutionError::IoError {
            message: error.to_string(),
            // os_code: error.raw_os_error(),
        }
    }
}

impl From<wasmtime::Error> for ExecutionError {
    fn from(error: wasmtime::Error) -> Self {
        ExecutionError::WasmError {
            message: error.to_string(),
        }
    }
}

impl From<RedisError> for ExecutionError {
    fn from(error: RedisError) -> Self {
        ExecutionError::RedisError {
            message: error.to_string(),
        }
    }
}

#[macro_export]
macro_rules! handle_dangling_error {
    ($e:expr) => {
        match $e {
            Ok(val) => val,
            Err(err) => {
                warn!(
                    "Unexpected dangling error in {}:{} error was {:?}",
                    file!(),
                    line!(),
                    err
                );
            }
        }
    };
}

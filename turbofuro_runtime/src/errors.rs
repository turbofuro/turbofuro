use redis::RedisError;
use serde::{Deserialize, Serialize};
use tel::{Description, StorageValue, TelError};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "code", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionError {
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
}

impl From<TelError> for ExecutionError {
    fn from(error: TelError) -> Self {
        ExecutionError::Tel { error }
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

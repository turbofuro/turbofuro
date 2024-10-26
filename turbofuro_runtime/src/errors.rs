use std::{error::Error, fmt::Display};

use redis::RedisError;
use serde::{Deserialize, Serialize};
use tel::{storage_value, Description, ObjectBody, StorageValue, TelError};
use tokio::sync::mpsc::error::SendError;

use crate::actor::ActorCommand;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorRepresentation {
    code: String,
    message: String,
    details: Option<StorageValue>,
    metadata: Option<StorageValue>,
}

impl ErrorRepresentation {
    pub fn to_value(&self) -> StorageValue {
        let mut map = ObjectBody::new();
        map.insert("code".to_owned(), self.code.clone().into());
        map.insert("message".to_owned(), self.message.clone().into());
        if let Some(details) = &self.details {
            map.insert("details".to_owned(), details.clone());
        }
        if let Some(metadata) = &self.metadata {
            map.insert("metadata".to_owned(), metadata.clone());
        }
        StorageValue::Object(map)
    }
}

impl Display for ErrorRepresentation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Error {}: {}\ndetails: {:?}\nmetadata: {:?}",
            self.code, self.message, self.details, self.metadata
        )
    }
}

impl From<TelError> for ErrorRepresentation {
    fn from(error: TelError) -> Self {
        match error {
            TelError::ParseError { .. } => ErrorRepresentation {
                code: "PARSE_ERROR".to_owned(),
                message: "Parse error".to_owned(),
                details: None,
                metadata: None,
            },
            TelError::ConversionError { message, from, to } => ErrorRepresentation {
                code: "CONVERSION_ERROR".to_owned(),
                message: format!("Conversion error: {} from {} to {}", message, from, to),
                details: None,
                metadata: None,
            },
            TelError::NotIndexable { message, subject } => ErrorRepresentation {
                code: "NOT_INDEXABLE".to_owned(),
                message,
                details: Some(storage_value!({
                    "subject": subject,
                })),
                metadata: None,
            },
            TelError::NoAttribute {
                message,
                subject,
                attribute,
            } => ErrorRepresentation {
                code: "NO_ATTRIBUTE".to_owned(),
                message,
                details: Some(storage_value!({
                    "subject": subject,
                    "attribute": attribute,
                })),
                metadata: None,
            },
            TelError::InvalidSelector { message } => ErrorRepresentation {
                code: "INVALID_SELECTOR".to_owned(),
                message,
                details: None,
                metadata: None,
            },
            TelError::UnsupportedOperation { operation, message } => ErrorRepresentation {
                code: "UNSUPPORTED_OPERATION".to_owned(),
                message,
                details: Some(storage_value!({
                    "operation": operation,
                })),
                metadata: None,
            },
            TelError::FunctionNotFound(name) => ErrorRepresentation {
                code: "FUNCTION_NOT_FOUND".to_owned(),
                message: "Function not found".to_owned(),
                details: Some(storage_value!({
                    "name": name,
                })),
                metadata: None,
            },
            TelError::IndexOutOfBounds { index, max } => ErrorRepresentation {
                code: "INDEX_OUT_OF_BOUNDS".to_owned(),
                message: "Index out of bounds".to_owned(),
                details: Some(storage_value!({
                    "index": index,
                    "max": max,
                })),
                metadata: None,
            },
            TelError::InvalidIndex { subject, message } => ErrorRepresentation {
                code: "INVALID_INDEX".to_owned(),
                message,
                details: Some(storage_value!({
                    "subject": subject,
                })),
                metadata: None,
            },
            TelError::InvalidArgument {
                index,
                method_name,
                expected,
                actual,
            } => ErrorRepresentation {
                code: "INVALID_ARGUMENT".to_owned(),
                message: format!("Invalid argument {} of {}", index, method_name),
                details: Some(storage_value!({
                    "index": index,
                    "name": method_name,
                    "expected": expected,
                    "actual": actual,
                })),
                metadata: None,
            },
            TelError::MissingArgument { index, method_name } => ErrorRepresentation {
                code: "MISSING_ARGUMENT".to_owned(),
                message: format!("Missing argument {} of {}", index, method_name),
                details: Some(storage_value!({
                    "index": index,
                    "name": method_name,
                })),
                metadata: None,
            },
        }
    }
}

impl From<ExecutionError> for ErrorRepresentation {
    fn from(error: ExecutionError) -> Self {
        match error {
            ExecutionError::Continue => ErrorRepresentation {
                code: "CONTINUE".to_owned(),
                message: "Continue".to_owned(),
                details: None,
                metadata: None,
            },
            ExecutionError::Break => ErrorRepresentation {
                code: "BREAK".to_owned(),
                message: "Break".to_owned(),
                details: None,
                metadata: None,
            },
            ExecutionError::Return { value } => ErrorRepresentation {
                code: "RETURN".to_owned(),
                message: "Return".to_owned(),
                details: Some(storage_value!({
                    "value": value.clone()
                })),
                metadata: None,
            },
            ExecutionError::Custom {
                inner_code,
                message,
                details,
                metadata,
            } => ErrorRepresentation {
                code: inner_code,
                message,
                details,
                metadata,
            },
            ExecutionError::Tel { error } => error.into(),
            ExecutionError::MissingParameter { name } => ErrorRepresentation {
                code: "MISSING_PARAMETER".to_owned(),
                message: format!("Missing parameter {}", name),
                details: Some(storage_value!({
                    "name": name,
                })),
                metadata: None,
            },
            ExecutionError::ParameterTypeMismatch {
                name,
                expected,
                actual,
            } => ErrorRepresentation {
                code: "PARAMETER_TYPE_MISMATCH".to_owned(),
                message: format!("Parameter {} has invalid type", name),
                details: Some(storage_value!({
                    "name": name,
                    "expected": expected,
                    "actual": actual,
                })),
                metadata: None,
            },
            ExecutionError::ParameterInvalid { name, message } => ErrorRepresentation {
                code: "PARAMETER_INVALID".to_owned(),
                message,
                details: Some(storage_value!({
                    "name": name,
                })),
                metadata: None,
            },
            ExecutionError::Unknown { message } => ErrorRepresentation {
                code: "UNKNOWN".to_owned(),
                message: format!("Unknown error: {}", message),
                details: None,
                metadata: None,
            },
            ExecutionError::StateInvalid {
                message,
                subject,
                inner,
            } => ErrorRepresentation {
                code: "STATE_INVALID".to_owned(),
                message,
                details: Some(storage_value!({
                    "subject": subject,
                    "inner": inner,
                })),
                metadata: None,
            },
            ExecutionError::MissingResource { resource_type } => ErrorRepresentation {
                code: "MISSING_RESOURCE".to_owned(),
                message: format!("Missing resource: {}", resource_type),
                details: Some(storage_value!({
                    "resource": resource_type,
                })),
                metadata: None,
            },
            ExecutionError::HandlerNotFound { name } => ErrorRepresentation {
                code: "HANDLER_NOT_FOUND".to_owned(),
                message: "Handler not found".to_owned(),
                details: Some(storage_value!({
                    "name": name,
                })),
                metadata: None,
            },
            ExecutionError::FunctionNotFound { id } => ErrorRepresentation {
                code: "FUNCTION_NOT_FOUND".to_owned(),
                message: "Function not found".to_owned(),
                details: Some(storage_value!({
                    "id": id,
                })),
                metadata: None,
            },
            ExecutionError::ModuleVersionNotFound { id } => ErrorRepresentation {
                code: "MODULE_VERSION_NOT_FOUND".to_owned(),
                message: "Module version not found".to_owned(),
                details: Some(storage_value!({
                    "id": id,
                })),
                metadata: None,
            },
            ExecutionError::UnresolvedImport { import_name } => ErrorRepresentation {
                code: "UNRESOLVED_IMPORT".to_owned(),
                message: "Unresolved import".to_string(),
                details: Some(storage_value!({
                    "import": import_name,
                })),
                metadata: None,
            },
            ExecutionError::Unsupported { message } => ErrorRepresentation {
                code: "UNSUPPORTED".to_owned(),
                message,
                details: None,
                metadata: None,
            },
            ExecutionError::SerializationFailed {
                message,
                breadcrumbs,
                inner,
            } => ErrorRepresentation {
                code: "SERIALIZATION_FAILED".to_owned(),
                message,
                details: Some(storage_value!({
                    "breadcrumbs": breadcrumbs,
                    "inner": inner,
                })),
                metadata: None,
            },
            ExecutionError::JwtDecodingFailed { message } => ErrorRepresentation {
                code: "JWT_DECODING_FAILED".to_owned(),
                message,
                details: None,
                metadata: None,
            },
            ExecutionError::ActorCommandFailed { message } => ErrorRepresentation {
                code: "ACTOR_COMMAND_FAILED".to_owned(),
                message,
                details: None,
                metadata: None,
            },
            ExecutionError::PostgresError { message, stage } => ErrorRepresentation {
                code: "POSTGRES_ERROR".to_owned(),
                message,
                details: Some(storage_value!({
                    "stage": stage,
                })),
                metadata: None,
            },
            ExecutionError::RedisError { message } => ErrorRepresentation {
                code: "REDIS_ERROR".to_owned(),
                message,
                details: None,
                metadata: None,
            },
            ExecutionError::IoError { message, os_code } => ErrorRepresentation {
                code: "IO_ERROR".to_owned(),
                message,
                details: os_code.map(|code| storage_value!({ "code": code })),
                metadata: None,
            },
            ExecutionError::WasmError { message } => ErrorRepresentation {
                code: "WASM_ERROR".to_owned(),
                message,
                details: None,
                metadata: None,
            },
            ExecutionError::WatcherError { message } => ErrorRepresentation {
                code: "WATCHER_ERROR".to_owned(),
                message,
                details: None,
                metadata: None,
            },
            ExecutionError::ParseError { message } => ErrorRepresentation {
                code: "PARSE_ERROR".to_owned(),
                message,
                details: None,
                metadata: None,
            },
            ExecutionError::LibSqlError { message, stage } => ErrorRepresentation {
                code: "LIBSQL_ERROR".to_owned(),
                message,
                details: Some(storage_value!({
                    "stage": stage
                })),
                metadata: None,
            },
            ExecutionError::ImageError { message } => ErrorRepresentation {
                code: "IMAGE_ERROR".to_owned(),
                message,
                details: None,
                metadata: None,
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "code", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionError {
    Continue,
    Break,
    Return {
        value: StorageValue,
    },
    Custom {
        inner_code: String,
        message: String,
        details: Option<StorageValue>,
        metadata: Option<StorageValue>,
    },
    Tel {
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
    LibSqlError {
        message: String,
        stage: String,
    },
    IoError {
        message: String,
        os_code: Option<i32>,
    },
    WasmError {
        message: String,
    },
    WatcherError {
        message: String,
    },
    ParseError {
        message: String,
    },
    ImageError {
        message: String,
    },
}

impl Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        ErrorRepresentation::from(self.clone()).fmt(f)
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
            os_code: error.raw_os_error(),
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

impl From<image::ImageError> for ExecutionError {
    fn from(error: image::ImageError) -> Self {
        ExecutionError::ImageError {
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

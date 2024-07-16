use crate::executor::{ExecutionEvent, ExecutionReport, ExecutionStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tel::Description;
use tokio::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub enum LoggerMessage {
    Log(ExecutionReport),
}

pub type ExecutionLoggerHandle = Sender<LoggerMessage>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DebugMessage {
    StartReport {
        id: String,
        status: ExecutionStatus,
        function_id: String,
        function_name: String,
        initial_storage: Description,
        events: Vec<ExecutionEvent>,
        module_id: String,
        module_version_id: String,
        environment_id: String,
        started_at: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        finished_at: Option<u64>,
        metadata: Option<Value>,
    },
    AppendEventToReport {
        id: String,
        event: ExecutionEvent,
    },
    EndReport {
        id: String,
        finished_at: u64,
        status: ExecutionStatus,
    },
}

pub type DebuggerHandle = Sender<DebugMessage>;

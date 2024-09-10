use crate::executor::{ExecutionEvent, ExecutionReport, ExecutionStatus};
use serde_json::Value;
use tel::{Description, StorageValue};
use tokio::sync::{mpsc::Sender, oneshot};

#[derive(Debug, Clone)]
pub enum LoggerMessage {
    Log(ExecutionReport),
}

pub type ExecutionLoggerHandle = Sender<LoggerMessage>;

#[derive(Debug)]
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
    AskForInput {
        id: String,
        text: String,
        label: String,
        placeholder: String,
        sender: oneshot::Sender<StorageValue>,
    },
    ShowResult {
        id: String,
        value: StorageValue,
    },
    ShowNotification {
        id: String,
        text: String,
        variant: String,
    },
    PlaySound {
        id: String,
        sound: String,
    },
}

pub type DebuggerHandle = Sender<DebugMessage>;

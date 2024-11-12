use crate::executor::{ExecutionEvent, ExecutionReport, ExecutionStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tel::{Description, StorageValue};
use tokio::sync::{mpsc::Sender, oneshot};

#[derive(Debug, Clone)]
pub enum LoggerMessage {
    Log(ExecutionReport),
}

pub type ExecutionLoggerHandle = Sender<LoggerMessage>;

pub type DebugOptionId = u64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugOption {
    pub label: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub theme: Option<String>,
    pub value: StorageValue,
}

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
    AskForValue {
        id: String,
        title: Option<String>,
        text: String,
        label: Option<String>,
        placeholder: Option<String>,
        options: Option<Vec<DebugOption>>,
        value: Option<StorageValue>,
        sender: oneshot::Sender<StorageValue>,
        mode: String,
    },
    ShowResult {
        id: String,
        title: Option<String>,
        text: Option<String>,
        value: Option<StorageValue>,
        variant: Option<String>,
    },
    ShowNotification {
        id: String,
        text: String,
        variant: Option<String>,
    },
    PlaySound {
        id: String,
        sound: String,
    },
}

pub type DebuggerHandle = Sender<DebugMessage>;

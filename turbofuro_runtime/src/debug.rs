use crate::executor::{ExecutionEvent, ExecutionReport, ExecutionStatus};
use serde::{Deserialize, Serialize};
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
        started_at: u64,
        initial_storage: Description,
    },
    AppendEvent {
        event: ExecutionEvent,
    },
    EndReport {
        finished_at: u64,
        status: ExecutionStatus,
    },
}

pub type DebuggerHandle = Sender<DebugMessage>;

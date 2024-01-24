use crate::executor::ExecutionReport;
use tokio::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub enum LoggerMessage {
    Log(ExecutionReport),
}

pub type ExecutionLoggerHandle = Sender<LoggerMessage>;

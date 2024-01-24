use std::collections::HashMap;

use reqwest::Client;
use tokio::sync::mpsc::{self};
use tracing::{debug, error};
use turbofuro_runtime::{
    execution_logging::{ExecutionLoggerHandle, LoggerMessage},
    executor::{ExecutionReport, ExecutionStatus},
};

#[derive(Debug, Clone, Default)]
struct LoggerStats {
    count: u64,
    errored: u64,
}

fn check_if_should_report(report: &ExecutionReport, stats: &mut LoggerStats) -> bool {
    let is_errored = matches!(report.log.status, ExecutionStatus::Failed);
    stats.count += 1;
    if is_errored {
        stats.errored += 1;
    }

    if is_errored {
        if stats.errored < 5 {
            true
        } else if stats.errored % 10 == 0 {
            fastrand::bool()
        } else {
            false
        }
    } else if stats.count < 5 {
        true
    } else if stats.count % 25 == 0 {
        fastrand::bool()
    } else {
        false
    }
}

pub static RUNS_ACCUMULATOR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn start_cloud_logger(token: String) -> ExecutionLoggerHandle {
    let client: Client = Client::new();
    let mut log_counter = HashMap::<String, LoggerStats>::new();

    let (sender, mut receiver) = mpsc::channel::<LoggerMessage>(16);
    tokio::spawn(async move {
        while let Some(log) = receiver.recv().await {
            RUNS_ACCUMULATOR.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match log {
                LoggerMessage::Log(report) => {
                    let should_report = {
                        let stats = log_counter.get_mut(&report.module_version_id);
                        if let Some(stats) = stats {
                            check_if_should_report(&report, stats)
                        } else {
                            let is_errored = matches!(report.log.status, ExecutionStatus::Failed);

                            // Always report the first report
                            log_counter.insert(
                                report.module_version_id.clone(),
                                LoggerStats {
                                    count: 1,
                                    errored: match is_errored {
                                        true => 1,
                                        false => 0,
                                    },
                                },
                            );
                            true
                        }
                    };

                    if should_report {
                        match client
                            .post("https://api.turbofuro.com/mission-control/log")
                            .header("x-turbofuro-token", &token)
                            .json(&report)
                            .send()
                            .await
                        {
                            Ok(response) => match response.text().await {
                                Ok(data) => {
                                    debug!("Cloud logger: Sent log {}", data);
                                }
                                Err(e) => {
                                    error!("Cloud logger: Failed to retrieve report ID {:?}", e)
                                }
                            },
                            Err(e) => {
                                error!(
                                    "Cloud logger: Failed to send log error was {:?} log was\n{}",
                                    e,
                                    serde_json::to_string(&report).unwrap()
                                );
                            }
                        }
                    } else {
                        debug!("Cloud logger: Skipping log")
                    }
                }
            }
        }
    });
    sender
}

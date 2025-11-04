use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::{
    cloud::agent::CloudAgentHandle,
    shared::{WorkerStoppingReason, WorkerWarning},
};

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    WorkerStarting,
    WorkerStarted,
    WorkerStopping(WorkerStoppingReason),
    WorkerStopped(WorkerStoppingReason),
    WarningRaised(Box<WorkerWarning>),
}

#[derive(Debug)]
pub struct WorkerEventReceiver(broadcast::Receiver<WorkerEvent>);

impl WorkerEventReceiver {
    pub async fn recv(&mut self) -> Result<WorkerEvent, ()> {
        self.0.recv().await.map_err(|_| ())
    }
}

impl Clone for WorkerEventReceiver {
    fn clone(&self) -> Self {
        WorkerEventReceiver(self.0.resubscribe())
    }
}

#[derive(Debug, Clone)]
pub struct WorkerEventSender(broadcast::Sender<WorkerEvent>);

impl WorkerEventSender {
    pub async fn send(&self, event: WorkerEvent) {
        match self.0.send(event) {
            Ok(_) => {}
            Err(err) => {
                debug!("Could not send worker event: {}", err);
            }
        }
    }
}

impl WorkerEvent {
    pub fn create_channel() -> (WorkerEventSender, WorkerEventReceiver) {
        let (sender, receiver) = broadcast::channel::<WorkerEvent>(16);
        (WorkerEventSender(sender), WorkerEventReceiver(receiver))
    }
}

pub fn spawn_console_observer(mut receiver: WorkerEventReceiver) {
    tokio::spawn(async move {
        while let Ok(event) = receiver.recv().await {
            match event {
                WorkerEvent::WorkerStarting => {
                    info!("Worker starting...");
                }
                WorkerEvent::WorkerStarted => {
                    info!("Worker started");
                }
                WorkerEvent::WorkerStopping(reason) => {
                    info!("Worker stopping: {:?}", reason);
                }
                WorkerEvent::WorkerStopped(reason) => {
                    info!("Worker stopped: {:?}", reason);
                }
                WorkerEvent::WarningRaised(warning) => {
                    warn!("Worker warning raised: {:?}", warning);
                }
            }
        }
    });
}

pub fn spawn_cloud_agent_observer(
    mut receiver: WorkerEventReceiver,
    cloud_agent: CloudAgentHandle,
) {
    tokio::spawn(async move {
        while let Ok(event) = receiver.recv().await {
            cloud_agent.handle_worker_event(event).await;
        }
    });
}

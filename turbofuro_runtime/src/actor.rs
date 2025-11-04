use std::collections::HashMap;
use std::fmt::Display;
use std::sync::Arc;
use std::vec;
use tel::describe;
use tel::ObjectBody;
use tel::StorageValue;
use tel::NULL;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::trace;
use tracing::warn;
use uuid::Uuid;

use crate::actions::alarms::cancellation_name;
use crate::debug::LoggerMessage;
use crate::errors::ExecutionError;
use crate::executor::execute;
use crate::executor::CompiledModule;
use crate::executor::DebuggerHandle;
use crate::executor::Environment;
use crate::executor::ExecutionContext;
use crate::executor::ExecutionLog;
use crate::executor::ExecutionMode;
use crate::executor::ExecutionReport;
use crate::executor::ExecutionStatus;
use crate::executor::Function;
use crate::executor::Global;
use crate::resources::ActorLink;
use crate::resources::ActorResources;

#[derive(Debug)]
pub enum ActorCommand {
    // TODO: Try convert those first 3 to one
    Run {
        handler: String,
        storage: ObjectBody,
        references: HashMap<String, String>,
        sender: Option<oneshot::Sender<Result<StorageValue, ExecutionError>>>,
        execution_id: Option<String>,
    },
    RunFunctionRef {
        function_ref: String,
        storage: ObjectBody,
        references: HashMap<String, String>,
        sender: Option<oneshot::Sender<Result<StorageValue, ExecutionError>>>,
        execution_id: Option<String>,
    },
    RunAlarm {
        handler: String,
        storage: ObjectBody,
        references: HashMap<String, String>,
        alarm_id: u64,
    },
    TakeResources(ActorResources),
    EnableDebugger {
        handle: DebuggerHandle,
    },
    DisableDebugger,
    Terminate,
}

impl Display for ActorCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActorCommand::Run { handler, .. } => write!(f, "Run handler {handler}"),
            ActorCommand::RunFunctionRef { function_ref, .. } => {
                write!(f, "Run function ref {function_ref}")
            }
            ActorCommand::RunAlarm { handler, .. } => {
                write!(f, "Run alarm for handler {handler}")
            }
            ActorCommand::TakeResources(_resources) => write!(f, "Take resources"),
            ActorCommand::EnableDebugger { handle: _ } => {
                write!(f, "Enable debugger")
            }
            ActorCommand::DisableDebugger => write!(f, "Disable debugger"),
            ActorCommand::Terminate => write!(f, "Terminate"),
        }
    }
}

/**
 * Runs service execution in a separate task
 */
pub fn activate_actor(mut actor: Actor) -> ActorLink {
    let module_id = actor.module.id.clone();
    let (sender, mut receiver) = mpsc::channel::<ActorCommand>(16);
    tokio::spawn(async move {
        while let Some(command) = receiver.recv().await {
            match command {
                ActorCommand::Run {
                    handler,
                    storage,
                    references,
                    sender,
                    execution_id,
                } => match actor
                    .execute_handler(&handler, storage, references, execution_id)
                    .await
                {
                    Ok(log) => {
                        match log.status {
                            ExecutionStatus::Finished => {
                                trace!("{} handled successfully, actor: {}", handler, actor.id);
                            }
                            ExecutionStatus::Failed => {
                                warn!(
                                    "{} handler failed, actor: {}. Execution log:\n{}",
                                    handler,
                                    actor.id,
                                    serde_json::to_string(&log).unwrap()
                                );
                            }
                            ExecutionStatus::Started => {
                                warn!(
                                    "{} handler returned started status, actor: {}. Execution log:\n{}",
                                    handler,
                                    actor.id,
                                    serde_json::to_string(&log).unwrap()
                                );
                            }
                        }

                        // If there is a sender, send the result back
                        if let Some(sender) = sender {
                            let payload = log.result.clone().unwrap_or(Ok(NULL));
                            match sender.send(payload) {
                                Ok(_) => {}
                                Err(_) => {
                                    warn!("Failed to report back execution of actor: {}", actor.id)
                                }
                            }
                        }

                        match actor.global.execution_logger.try_send(LoggerMessage::Log(
                            ExecutionReport {
                                module_id: actor.module.module_id.clone(),
                                module_version_id: actor.module.id.clone(),
                                environment_id: actor.environment.id.clone(),
                                metadata: None,
                                status: log.status,
                                function_id: log.function_id,
                                function_name: log.function_name,
                                initial_storage: log.initial_storage,
                                events: log.events,
                                started_at: log.started_at,
                                finished_at: log.finished_at,
                            },
                        )) {
                            Ok(_) => {}
                            Err(e) => {
                                warn!(
                                        "Failed to send execution after handling {}, actor: {} log: {:?}",
                                        handler, actor.id, e
                                    );
                            }
                        }
                    }
                    Err(e) => {
                        info!(
                            "Could not handle {}, actor: {} message: {:?}",
                            handler, actor.id, e
                        );

                        // By common sense the execution error should not be a return here
                        if let Some(sender) = sender {
                            match sender.send(Err(e)) {
                                Ok(_) => {}
                                Err(err) => {
                                    warn!(
                                        "Failed to report back execution of actor: {} error: {:?}",
                                        actor.id, err
                                    )
                                }
                            }
                        }
                    }
                },
                ActorCommand::RunFunctionRef {
                    function_ref,
                    storage,
                    references,
                    sender,
                    execution_id,
                } => match actor
                    .execute_function(&function_ref, storage, references, execution_id)
                    .await
                {
                    Ok(log) => {
                        match log.status {
                            ExecutionStatus::Finished => {
                                trace!(
                                    "{} ref/handled successfully, actor: {}",
                                    function_ref,
                                    actor.id
                                );
                            }
                            ExecutionStatus::Failed => {
                                warn!(
                                    "ref/{} handler failed, actor: {}. Execution log:\n{}",
                                    function_ref,
                                    actor.id,
                                    serde_json::to_string(&log).unwrap()
                                );
                            }
                            ExecutionStatus::Started => {
                                warn!(
                                    "ref/{} handler returned started status, actor: {}. Execution log:\n{}",
                                    function_ref,
                                    actor.id,
                                    serde_json::to_string(&log).unwrap()
                                );
                            }
                        }

                        // If there is a sender, send the result back
                        if let Some(sender) = sender {
                            let payload = log.result.clone().unwrap_or(Ok(NULL));
                            match sender.send(payload) {
                                Ok(_) => {}
                                Err(_) => {
                                    warn!("Failed to report back execution of actor: {}", actor.id)
                                }
                            }
                        }

                        match actor.global.execution_logger.try_send(LoggerMessage::Log(
                            ExecutionReport {
                                module_id: actor.module.module_id.clone(),
                                module_version_id: actor.module.id.clone(),
                                environment_id: actor.environment.id.clone(),
                                metadata: None,
                                status: log.status,
                                function_id: log.function_id,
                                function_name: log.function_name,
                                initial_storage: log.initial_storage,
                                events: log.events,
                                started_at: log.started_at,
                                finished_at: log.finished_at,
                            },
                        )) {
                            Ok(_) => {}
                            Err(e) => {
                                warn!(
                                        "Failed to send execution after handling ref/{}, actor: {} log: {:?}",
                                        function_ref, actor.id, e
                                    );
                            }
                        }
                    }
                    Err(e) => {
                        info!(
                            "Could not handle ref/{}, actor: {} message: {:?}",
                            function_ref, actor.id, e
                        );

                        match e {
                            ExecutionError::Return { value } => {
                                if let Some(sender) = sender {
                                    match sender.send(Ok(value)) {
                                        Ok(_) => {}
                                        Err(err) => {
                                            warn!(
                                                "Failed to report back execution of actor: {} error: {:?}",
                                                actor.id, err
                                            )
                                        }
                                    }
                                }
                            }
                            _ => {
                                if let Some(sender) = sender {
                                    match sender.send(Err(e)) {
                                        Ok(_) => {}
                                        Err(err) => {
                                            warn!(
                                                "Failed to report back execution of actor: {} error: {:?}",
                                                actor.id, err
                                            )
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                ActorCommand::RunAlarm {
                    handler,
                    storage,
                    references,
                    alarm_id,
                } => {
                    // Remove alarm in place from actor resources (not cloneable)
                    let _cancellation = actor
                        .resources
                        .pop_cancellation_where(|c| c.name == cancellation_name(alarm_id));

                    match actor
                        .execute_handler(&handler, storage, references, None)
                        .await
                    {
                        Ok(log) => {
                            match log.status {
                                ExecutionStatus::Finished => {
                                    trace!("{} handled successfully, actor: {}", handler, actor.id);
                                }
                                ExecutionStatus::Failed => {
                                    warn!(
                                        "{} handler failed, actor: {}. Execution log:\n{}",
                                        handler,
                                        actor.id,
                                        serde_json::to_string(&log).unwrap()
                                    );
                                }
                                ExecutionStatus::Started => {
                                    warn!(
                                        "{} handler returned started status, actor: {}. Execution log:\n{}",
                                        handler,
                                        actor.id,
                                        serde_json::to_string(&log).unwrap()
                                    );
                                }
                            }

                            match actor.global.execution_logger.try_send(LoggerMessage::Log(
                                ExecutionReport {
                                    module_id: actor.module.module_id.clone(),
                                    module_version_id: actor.module.id.clone(),
                                    environment_id: actor.environment.id.clone(),
                                    metadata: None,
                                    status: log.status,
                                    function_id: log.function_id,
                                    function_name: log.function_name,
                                    initial_storage: log.initial_storage,
                                    events: log.events,
                                    started_at: log.started_at,
                                    finished_at: log.finished_at,
                                },
                            )) {
                                Ok(_) => {}
                                Err(e) => {
                                    warn!(
                                        "Failed to send execution after handling {}, actor: {} log: {:?}",
                                        handler, actor.id, e
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            info!(
                                "Could not handle {}, actor: {} message: {:?}",
                                handler, actor.id, e
                            );
                        }
                    }
                }
                ActorCommand::Terminate => {
                    actor.terminate();
                }
                ActorCommand::TakeResources(mut resources) => {
                    actor.resources.append(&mut resources);
                }
                ActorCommand::EnableDebugger { handle } => {
                    actor.debugger = Some(handle);
                }
                ActorCommand::DisableDebugger => {
                    actor.debugger = None;
                }
            }
        }
    });

    ActorLink::new(sender, module_id)
}

pub fn spawn_ok_or_terminate(
    actor_link: ActorLink,
    response_receiver: oneshot::Receiver<Result<StorageValue, ExecutionError>>,
) {
    tokio::spawn(async move {
        match response_receiver.await {
            Ok(resp) => {
                match resp {
                    Ok(_) => {
                        // Do nothing
                    }
                    Err(_e) => {
                        // If the execution has failed we should terminate the actor
                        match actor_link.send(ActorCommand::Terminate).await {
                            Ok(_) => {}
                            Err(_) => warn!("Failed to terminate actor after errored response"),
                        }
                    }
                }
            }
            Err(e) => {
                error!(
                    "Error receiving response from actor (run response): {:?}",
                    e
                );
            }
        }
    });
}

#[derive(Debug)]
pub struct Actor {
    id: String,
    state: StorageValue,
    module: Arc<CompiledModule>,
    global: Arc<Global>,
    environment: Arc<Environment>,
    resources: ActorResources,
    handlers: HashMap<String, String>,
    debugger: Option<DebuggerHandle>,
}

impl Drop for Actor {
    fn drop(&mut self) {
        debug!("Actor {} dropped", self.id);
    }
}

impl Actor {
    pub fn new(
        state: StorageValue,
        environment: Arc<Environment>,
        module: Arc<CompiledModule>,
        global: Arc<Global>,
        resources: ActorResources,
        handlers: HashMap<String, String>,
        debugger: Option<DebuggerHandle>,
    ) -> Self {
        let id = nanoid::nanoid!();

        Self {
            id,
            state,
            module,
            global,
            environment,
            resources,
            handlers,
            debugger,
        }
    }

    pub fn get_id(&self) -> &str {
        &self.id
    }

    pub fn terminate(&mut self) {
        debug!(
            "Terminating actor id: {}, module: {}",
            self.id, self.module.id
        );

        // Clean up local resources
        while let Some(cancellation) = self.resources.pop_cancellation() {
            match cancellation.sender.send(()) {
                Ok(_) => {}
                Err(_) => {
                    warn!(
                        "Failed to cancel (name: {}) on actor: {}",
                        cancellation.name, self.id
                    );
                }
            }
        }

        self.resources = ActorResources::default();
        self.global.registry.actors.remove(&self.id);
    }

    pub async fn execute_handler(
        &mut self,
        handler_name: &str,
        initial_storage: ObjectBody,
        references: HashMap<String, String>,
        execution_id: Option<String>,
    ) -> Result<ExecutionLog, ExecutionError> {
        let handler_function_id = {
            match self.handlers.get(handler_name) {
                Some(id) => id.clone(),
                None => {
                    debug!(
                        "Handler {} not found, actor: {} mvid: {}",
                        handler_name,
                        self.get_id(),
                        self.module.id
                    );
                    return Err(ExecutionError::HandlerNotFound {
                        name: handler_name.into(),
                    });
                }
            }
        };

        self.execute_function(
            &handler_function_id,
            initial_storage,
            references,
            execution_id,
        )
        .await
    }

    pub async fn execute_function(
        &mut self,
        function_id: &str,
        initial_storage: ObjectBody,
        references: HashMap<String, String>,
        execution_id: Option<String>,
    ) -> Result<ExecutionLog, ExecutionError> {
        let function = self.module.get_function(function_id)?;

        let mut storage = initial_storage;
        storage.insert("state".to_owned(), self.state.clone());

        // Build execution context
        let mut context = ExecutionContext {
            id: execution_id.unwrap_or_else(|| Uuid::now_v7().to_string()),
            actor_id: self.get_id().to_owned(),
            log: ExecutionLog::new_started(
                describe(StorageValue::Object(storage.clone())),
                function_id,
                function.get_name(),
            ),
            storage,
            environment: self.environment.clone(),
            resources: &mut self.resources,
            module: self.module.clone(),
            global: self.global.clone(),
            bubbling: false,
            references,
            mode: match &self.debugger {
                Some(handle) => ExecutionMode::Debug(handle.clone()),
                None => ExecutionMode::Probe, // TODO: Roll the dice or something to prefer fast mode
            },
            loop_counts: vec![],
        };

        let body = match &function {
            Function::Normal { body, .. } => body,
            Function::Native { id, .. } => {
                // TODO: Handle native functions
                return Err(ExecutionError::Unsupported {
                    message: format!("Native function {id} can't be executed as a handler"),
                });
            }
        };

        context.start_report().await;

        Ok(match execute(body, &mut context).await {
            Ok(_) => {
                self.state = context
                    .storage
                    .get("state")
                    .unwrap_or(&StorageValue::Null(None))
                    .clone();

                debug!("Execution finished successfully");
                context.end_report(ExecutionStatus::Finished).await;
                // TODO: Shall we return the result here?
                // context.log.result = Some(Ok(NULL));
                context.log
            }
            Err(e) => match e {
                ExecutionError::Return { value } => {
                    self.state = context
                        .storage
                        .get("state")
                        .unwrap_or(&StorageValue::Null(None))
                        .clone();

                    debug!("Execution finished successfully (return)");
                    context.end_report(ExecutionStatus::Finished).await;

                    context.log.result = Some(Ok(value));
                    context.log
                }
                e => {
                    warn!("Execution failed: error: ${:?}", e);
                    context.end_report(ExecutionStatus::Failed).await;

                    context.log.status = ExecutionStatus::Failed;
                    context.log.result = Some(Err(e));
                    context.log
                }
            },
        })
    }
}

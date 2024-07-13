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
use crate::executor::Steps;
use crate::resources::ActorLink;
use crate::resources::ActorResources;

#[derive(Debug)]
pub enum ActorCommand {
    // TODO: Try convert those first 3 to one
    Run {
        handler: String,
        storage: ObjectBody,
        sender: Option<oneshot::Sender<Result<StorageValue, ExecutionError>>>, // TODO: Add action reply
    },
    RunFunctionRef {
        function_ref: String,
        storage: ObjectBody,
        sender: Option<oneshot::Sender<Result<StorageValue, ExecutionError>>>, // TODO: Add action reply
    },
    RunAlarm {
        handler: String,
        storage: ObjectBody,
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
            ActorCommand::Run { handler, .. } => write!(f, "Run handler {}", handler),
            ActorCommand::RunFunctionRef { function_ref, .. } => {
                write!(f, "Run function ref {}", function_ref)
            }
            ActorCommand::RunAlarm { handler, .. } => {
                write!(f, "Run alarm for handler {}", handler)
            }
            ActorCommand::TakeResources(_resources) => write!(f, "Take resources"),
            ActorCommand::EnableDebugger { handle } => write!(f, "Enable debugger {}", handle.id),
            ActorCommand::DisableDebugger => write!(f, "Disable debugger"),
            ActorCommand::Terminate => write!(f, "Terminate"),
        }
    }
}

/**
 * Runs service execution in a separate task
 */
pub fn activate_actor(mut actor: Actor) -> ActorLink {
    let (sender, mut receiver) = mpsc::channel::<ActorCommand>(16);
    tokio::spawn(async move {
        while let Some(command) = receiver.recv().await {
            match command {
                ActorCommand::Run {
                    handler,
                    storage,
                    sender,
                } => match actor.execute_handler(&handler, storage).await {
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
                                log,
                                metadata: None,
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
                    sender,
                } => match actor.execute_function_ref(&function_ref, storage).await {
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
                                log,
                                metadata: None,
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
                    alarm_id,
                } => {
                    // Remove alarm in place from actor resources (not cloneable)
                    let mut alarm = None;
                    for (i, a) in actor.resources.cancellations.iter().enumerate() {
                        if a.name == cancellation_name(alarm_id) {
                            alarm = Some(i);
                            break;
                        }
                    }
                    if let Some(i) = alarm {
                        actor.resources.cancellations.remove(i);
                    }

                    match actor.execute_handler(&handler, storage).await {
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
                                    log,
                                    metadata: None,
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
                    actor
                        .resources
                        .http_requests_to_respond
                        .append(&mut resources.http_requests_to_respond);
                    actor.resources.websockets.append(&mut resources.websockets);
                    actor
                        .resources
                        .cancellations
                        .append(&mut resources.cancellations);
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

    ActorLink(sender)
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
            debugger: None,
        }
    }

    pub fn new_module_initiator(
        state: StorageValue,
        environment: Arc<Environment>,
        module: Arc<CompiledModule>,
        global: Arc<Global>,
        resources: ActorResources,
    ) -> Self {
        let id = nanoid::nanoid!();

        Self {
            id,
            state,
            module: module.clone(),
            global,
            environment,
            resources,
            handlers: module.handlers.clone(),
            debugger: None,
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
        while let Some(cancellation) = self.resources.cancellations.pop() {
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

    pub async fn execute_custom(
        &mut self,
        steps: &Steps,
        mut resources: ActorResources,
        initial_storage: ObjectBody,
        debugger: DebuggerHandle,
    ) -> ExecutionLog {
        let mut storage = initial_storage;
        storage.insert("state".to_owned(), self.state.clone());

        // Build execution context
        let mut context = ExecutionContext {
            actor_id: self.get_id().to_owned(),
            log: ExecutionLog::started_with_initial_storage(
                StorageValue::Object(storage.clone()).into(),
            ),
            storage,
            environment: self.environment.clone(),
            resources: &mut resources,
            module: self.module.clone(),
            global: self.global.clone(),
            bubbling: false,
            references: HashMap::new(),
            mode: ExecutionMode::Debug(debugger),
            loop_counts: vec![],
        };

        match execute(steps, &mut context).await {
            Ok(_) => {
                self.state = context
                    .storage
                    .get("state")
                    .unwrap_or(&StorageValue::Null(None))
                    .clone();

                debug!("Execution finished successfully");
                context.log
            }
            Err(e) => match e {
                // Case where early return was called
                ExecutionError::Return { .. } => {
                    self.state = context
                        .storage
                        .get("state")
                        .unwrap_or(&StorageValue::Null(None))
                        .clone();

                    debug!("Execution finished successfully (return)");
                    context.log
                }
                e => {
                    debug!("Execution failed: error: ${:?}", e);
                    context.log.status = ExecutionStatus::Failed;
                    context.log
                }
            },
        }
    }

    pub async fn execute_handler(
        &mut self,
        handler_name: &str,
        initial_storage: ObjectBody,
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

        self.execute_function_ref(&handler_function_id, initial_storage)
            .await
    }

    pub async fn execute_function_ref(
        &mut self,
        function_ref: &str,
        initial_storage: ObjectBody,
    ) -> Result<ExecutionLog, ExecutionError> {
        let function = self
            .module
            .exported_functions
            .iter()
            .find(|f| f.get_id() == function_ref)
            .or_else(|| {
                self.module
                    .local_functions
                    .iter()
                    .find(|f| f.get_id() == function_ref)
            })
            .ok_or(ExecutionError::FunctionNotFound {
                id: function_ref.to_owned(),
            })?;

        let mut storage = initial_storage;
        storage.insert("state".to_owned(), self.state.clone());

        // Build execution context
        let mut context = ExecutionContext {
            actor_id: self.get_id().to_owned(),
            log: ExecutionLog::started_with_initial_storage(describe(StorageValue::Object(
                storage.clone(),
            ))),
            storage,
            environment: self.environment.clone(),
            resources: &mut self.resources,
            module: self.module.clone(),
            global: self.global.clone(),
            bubbling: false,
            references: HashMap::new(),
            mode: match &self.debugger {
                Some(handle) => ExecutionMode::Debug(handle.clone()),
                None => ExecutionMode::Probe, // TODO: Roll the dice or something to prefer fast mode
            },
            loop_counts: vec![],
        };

        let body = match &function {
            Function::Normal { body, .. } => body,
            Function::Native { id, .. } => {
                return Err(ExecutionError::Unsupported {
                    message: format!("Native function {} can't be executed as a handler", id),
                });
            }
        };

        Ok(match execute(body, &mut context).await {
            Ok(_) => {
                self.state = context
                    .storage
                    .get("state")
                    .unwrap_or(&StorageValue::Null(None))
                    .clone();

                debug!("Execution finished successfully");
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
                    context.log.result = Some(Ok(value));
                    context.log
                }
                e => {
                    warn!("Execution failed: error: ${:?}", e);
                    context.log.status = ExecutionStatus::Failed;
                    context.log.result = Some(Err(e));
                    context.log
                }
            },
        })
    }
}

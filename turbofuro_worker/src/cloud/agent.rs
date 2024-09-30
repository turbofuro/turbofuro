use nanoid::nanoid;
use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::{
    cloud::operator_client::get_reported_debug_entries,
    config::{fetch_configuration, ConfigurationCoordinator},
    environment_resolver::SharedEnvironmentResolver,
    errors::WorkerError,
    events::WorkerEvent,
    module_version_resolver::{ModuleVersionResolver, SharedModuleVersionResolver},
    options::CloudOptions,
    shared::{install_module, ModuleVersion, WorkerStatus},
    VERSION,
};
use tokio::sync::{
    broadcast,
    mpsc::{self},
    oneshot,
};
use tracing::{debug, error, info, info_span, instrument, warn, Instrument};
use turbofuro_runtime::{
    actor::{activate_actor, Actor, ActorCommand},
    debug::DebugMessage,
    executor::{
        evaluate_parameters, get_timestamp, Callee, DebugState, DebuggerHandle, Environment,
        Global, Parameter,
    },
    resources::{ActorLink, ActorResources},
    ObjectBody, StorageValue,
};

use super::operator_client::{OperatorClientHandle, SendingCommand};

#[derive(Debug)]
enum CloudAgentMessage {
    Start,
    HandleWorkerEvent {
        event: WorkerEvent,
    },
    PerformRun {
        id: String,
        module_version: ModuleVersion,
        callee: Callee,
        parameters: Vec<Parameter>,
    },
    EnableDebugger {
        module_id: String,
        module_version: Option<ModuleVersion>,
    },
    DisableDebugger {
        module_id: String,
    },
    ReloadConfiguration,
    ReloadEnvironment,
    SetupDebugListener {
        id: String,
        sender: oneshot::Sender<StorageValue>,
    },
    FulfillDebugListener {
        id: String,
        value: StorageValue,
    },
}

#[derive(Debug)]
pub struct DebugListener {
    pub id: String,
    pub sender: oneshot::Sender<StorageValue>,
}

struct CloudAgent {
    // Main agent state
    debug_state: DebugState,
    debug_listeners: Vec<DebugListener>,
    status: WorkerStatus,
    operator_client: OperatorClientHandle,
    options: CloudOptions,

    // Worker stuff
    environment_resolver: SharedEnvironmentResolver,
    module_version_resolver: SharedModuleVersionResolver,
    configuration_coordinator: ConfigurationCoordinator,
    global: Arc<Global>,

    // The main receiver, which we don't have sender for as it owning the agent
    main_receiver: mpsc::Receiver<CloudAgentMessage>,

    worker_reload_sender: broadcast::Sender<()>,

    // The child receiver, which we have sender for, never dropped, but used to let operator client pass messages back to the agent
    child_receiver: mpsc::Receiver<CloudAgentMessage>,
    child_handle: CloudAgentHandle,
}

fn spawn_debugger_handle_reader(
    mut receiver: tokio::sync::mpsc::Receiver<DebugMessage>,
    operator_client: OperatorClientHandle,
    mut cloud_agent_handler: CloudAgentHandle,
    module_id: String,
) {
    tokio::spawn(async move {
        while let Some(message) = receiver.recv().await {
            match message {
                DebugMessage::AskForInput {
                    id,
                    text,
                    label,
                    placeholder,
                    sender,
                } => {
                    // Add id and sender to the debug state
                    cloud_agent_handler
                        .setup_debugger_listener(id.clone(), sender)
                        .await;

                    operator_client
                        .send_command(SendingCommand::DebugAction {
                            module_id: module_id.clone(),
                            action: crate::cloud::operator_client::DebugAction::AskForInput {
                                id,
                                text,
                                label,
                                placeholder,
                            },
                        })
                        .await;
                }
                DebugMessage::StartReport {
                    id,
                    status,
                    function_id,
                    function_name,
                    initial_storage,
                    events,
                    module_id,
                    module_version_id,
                    environment_id,
                    started_at,
                    metadata,
                } => {
                    operator_client
                        .send_command(SendingCommand::StartDebugReport {
                            id,
                            started_at,
                            initial_storage,
                            module_id,
                            module_version_id,
                            environment_id,
                            function_id,
                            function_name,
                            status,
                            events,
                            metadata,
                        })
                        .await;
                }
                DebugMessage::AppendEventToReport { id, event } => {
                    operator_client
                        .send_command(SendingCommand::AppendEventToDebugReport { id, event })
                        .await;
                }
                DebugMessage::EndReport {
                    id,
                    finished_at,
                    status,
                } => {
                    operator_client
                        .send_command(SendingCommand::EndDebugReport {
                            id,
                            status,
                            finished_at,
                        })
                        .await;
                }
                DebugMessage::ShowResult { id, value } => {
                    operator_client
                        .send_command(SendingCommand::DebugAction {
                            action: super::operator_client::DebugAction::ShowResult { id, value },
                            module_id: module_id.clone(),
                        })
                        .await;
                }
                DebugMessage::ShowNotification { id, text, variant } => {
                    operator_client
                        .send_command(SendingCommand::DebugAction {
                            action: super::operator_client::DebugAction::ShowNotification {
                                id,
                                text,
                                variant,
                            },
                            module_id: module_id.clone(),
                        })
                        .await;
                }
                DebugMessage::PlaySound { id, sound } => {
                    operator_client
                        .send_command(SendingCommand::DebugAction {
                            action: super::operator_client::DebugAction::PlaySound { id, sound },
                            module_id: module_id.clone(),
                        })
                        .await;
                }
            }
        }
    });
}

impl CloudAgent {
    async fn handle_message(&mut self, message: CloudAgentMessage) {
        debug!("Handling message: {:?}", message);
        match message {
            CloudAgentMessage::HandleWorkerEvent { event } => {
                match event {
                    WorkerEvent::WorkerStarting => {
                        // Make sure to clear warnings
                        self.status = WorkerStatus::Starting { warnings: vec![] };
                    }
                    WorkerEvent::WorkerStarted => {
                        self.status = WorkerStatus::Running {
                            warnings: self.status.get_warnings(),
                        };
                    }
                    WorkerEvent::WorkerStopping(reason) => {
                        self.status = WorkerStatus::Stopping {
                            warnings: self.status.get_warnings(),
                            reason,
                        };
                    }
                    WorkerEvent::WorkerStopped(reason) => {
                        self.status = WorkerStatus::Stopped {
                            warnings: self.status.get_warnings(),
                            reason,
                        };
                    }
                    WorkerEvent::WarningRaised(warning) => {
                        self.status.add_warning(warning.clone());
                    }
                }
                self.update_state().await
            }
            CloudAgentMessage::PerformRun {
                id,
                module_version,
                callee,
                parameters,
            } => {
                let (debugger_handle, receiver) = DebuggerHandle::new();
                spawn_debugger_handle_reader(
                    receiver,
                    self.operator_client.clone(),
                    self.child_handle.clone(),
                    module_version.module_id.clone(),
                );

                // TODO: Report error if the run could not be performed ie. could not resolve imported module
                let _ = self
                    .perform_debug_run(id, module_version, callee, parameters, debugger_handle)
                    .await;
            }
            CloudAgentMessage::EnableDebugger {
                module_id,
                module_version,
            } => {
                let (debugger_handle, receiver) = DebuggerHandle::new();
                spawn_debugger_handle_reader(
                    receiver,
                    self.operator_client.clone(),
                    self.child_handle.clone(),
                    module_id.clone(),
                );

                let mut should_reload_worker = false;
                let module = match module_version {
                    Some(module_version) => {
                        let module = install_module(
                            module_version,
                            self.global.clone(),
                            self.module_version_resolver.clone(),
                        )
                        .await
                        .unwrap(); // TODO: Handle errors

                        should_reload_worker = true;
                        Some(module)
                    }
                    None => None,
                };

                self.debug_state.add_or_update_entry(
                    module_id.clone(),
                    debugger_handle.clone(),
                    module,
                );

                self.global
                    .debug_state
                    .store(Arc::new(self.debug_state.clone()));

                if should_reload_worker {
                    self.worker_reload_sender.send(()).unwrap();
                } else {
                    let actors_to_message = self
                        .global
                        .registry
                        .actors
                        .iter()
                        .filter(|item| {
                            let actor_link = item.value();
                            actor_link.module_id == module_id
                        })
                        .map(|item| item.value().clone())
                        .collect::<Vec<ActorLink>>();

                    for actor_link in actors_to_message {
                        let _ = actor_link
                            .send(ActorCommand::EnableDebugger {
                                handle: debugger_handle.clone(),
                            })
                            .await;
                    }
                }

                self.update_state().await
            }
            CloudAgentMessage::DisableDebugger { module_id } => {
                let removed = self.debug_state.remove_entry(&module_id);
                if let Some(removed) = removed {
                    self.global
                        .debug_state
                        .store(Arc::new(self.debug_state.clone()));

                    let should_reload_worker = removed.module.is_some();
                    if should_reload_worker {
                        self.worker_reload_sender.send(()).unwrap();
                    } else {
                        let actors_to_message = self
                            .global
                            .registry
                            .actors
                            .iter()
                            .filter(|item| {
                                let actor_link = item.value();
                                actor_link.module_id == module_id
                            })
                            .map(|item| item.value().clone())
                            .collect::<Vec<ActorLink>>();

                        for actor_link in actors_to_message {
                            let _ = actor_link.send(ActorCommand::DisableDebugger {}).await;
                        }
                    }
                } else {
                    warn!(
                        "Debug entry not found, command was for module {}",
                        module_id.clone()
                    );
                }

                self.update_state().await
            }
            CloudAgentMessage::ReloadConfiguration => {
                info!("Reloading configuration");
                let result = fetch_configuration(&self.options).await;
                match result {
                    Ok(configuration) => {
                        self.configuration_coordinator
                            .update_configuration(configuration)
                            .await;
                    }
                    Err(e) => {
                        warn!("Failed to fetch configuration: {}", e);
                    }
                }
            }
            CloudAgentMessage::ReloadEnvironment => {
                let environment_id = self.global.environment.load().id.clone();

                let environment: Environment = match self
                    .environment_resolver
                    .lock()
                    .await
                    .get_environment(&environment_id)
                    .instrument(info_span!("get_environment"))
                    .await
                {
                    Ok(environment) => environment,
                    Err(e) => {
                        error!("Could not fetch environment: {:?}", e);
                        return;
                    }
                };

                self.global.environment.swap(Arc::new(environment));
                info!("Environment updated");
            }
            CloudAgentMessage::Start => self.operator_client.connect().await,
            CloudAgentMessage::SetupDebugListener { id, sender } => {
                self.debug_listeners.push(DebugListener { id, sender });
            }
            CloudAgentMessage::FulfillDebugListener { id, value } => {
                // Find and pop the listener
                if let Some(index) = self.debug_listeners.iter().position(|l| l.id == id) {
                    let listener = self.debug_listeners.swap_remove(index);

                    // Ignore errors, the listener might be gone already
                    let _ = listener.sender.send(value);
                }
            }
        }
    }

    async fn check_stale_debug(&mut self) {
        self.debug_state.remove_old_entries();
    }

    async fn update_state(&mut self) {
        self.operator_client
            .send_command(SendingCommand::UpdateState {
                version: VERSION,
                os: std::env::consts::OS,
                name: self.options.name.clone(),
                status: self.status.clone(),
                timestamp: get_timestamp(),
                debug: get_reported_debug_entries(&self.debug_state),
                rent: self.options.rent.clone(),
            })
            .await;
    }

    async fn handle_tick(&mut self) {
        self.check_stale_debug().await;

        // Let's update state every so often
        self.update_state().await
    }

    #[instrument(level = "info", skip_all)]
    async fn perform_debug_run(
        &mut self,
        id: String,
        module_version: ModuleVersion,
        callee: Callee,
        parameters: Vec<Parameter>,
        debugger: DebuggerHandle,
    ) -> Result<(), WorkerError> {
        let function_id = match callee {
            Callee::Local { function_id } => function_id,
            Callee::Import {
                import_name: _,
                function_id: _,
            } => {
                return Err(WorkerError::Unsupported {
                    message: "Can't perform custom run for imported functions".to_owned(),
                });
            }
        };

        let global: Arc<Global> = self.global.clone();
        let environment: Arc<Environment> = self.global.environment.load().clone();
        let module_version_resolver: Arc<dyn ModuleVersionResolver> =
            self.module_version_resolver.clone();
        let compiled_module: Arc<turbofuro_runtime::executor::CompiledModule> = install_module(
            module_version,
            global.clone(),
            module_version_resolver.clone(),
        )
        .await?;

        let actor = Actor::new(
            StorageValue::Null(None),
            environment.clone(),
            compiled_module.clone(),
            self.global.clone(),
            ActorResources::default(),
            HashMap::new(),
            Some(debugger),
        );
        let link = activate_actor(actor);

        let (storage, references) =
            evaluate_parameters(&parameters, &ObjectBody::new(), &environment)?;

        let (sender, receiver) = oneshot::channel();
        link.send(ActorCommand::RunFunctionRef {
            function_ref: function_id,
            storage,
            references,
            sender: Some(sender),
            execution_id: Some(id),
        })
        .await
        .map_err(WorkerError::from)?;

        tokio::spawn(async move {
            let _ = receiver.await;
            let _ = link.send(ActorCommand::Terminate).await;
        });

        return Ok(());
    }
}

async fn run_cloud_agent(mut agent: CloudAgent) {
    let mut timer = tokio::time::interval(Duration::from_secs(180));

    loop {
        tokio::select! {
            _ = timer.tick() => {
                agent.handle_tick().await;

            }
            child_message = agent.child_receiver.recv() => {
                match child_message {
                    Some(message) => agent.handle_message(message).await,
                    None => {
                        // Unlikely to happen since the agent has it's own sender
                        break;
                    }
                }
            },
            message = agent.main_receiver.recv() => {
                match message {
                    Some(message) => agent.handle_message(message).await,
                    None => {
                        break;
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct CloudAgentHandle {
    sender: mpsc::Sender<CloudAgentMessage>,
}

impl CloudAgentHandle {
    pub fn new(
        options: CloudOptions,
        global: Arc<Global>,
        module_version_resolver: SharedModuleVersionResolver,
        environment_resolver: SharedEnvironmentResolver,
        configuration_coordinator: ConfigurationCoordinator,
        worker_reload_sender: broadcast::Sender<()>,
    ) -> Result<Self, WorkerError> {
        let (sender, receiver) = mpsc::channel(16);
        let (child_sender, child_receiver) = mpsc::channel(16);

        let worker_id = nanoid!();
        let operator_url = options.get_operator_url(worker_id.clone())?;

        let child_handle = CloudAgentHandle {
            sender: child_sender.clone(),
        };

        let operator_client = OperatorClientHandle::new(operator_url, child_handle.clone());

        let actor = CloudAgent {
            options,
            debug_listeners: vec![],
            environment_resolver,
            module_version_resolver,
            configuration_coordinator,
            global,
            debug_state: DebugState::default(),
            status: WorkerStatus::Starting { warnings: vec![] },
            main_receiver: receiver,
            child_receiver,
            child_handle,
            operator_client,
            worker_reload_sender,
        };

        tokio::spawn(run_cloud_agent(actor));

        Ok(Self { sender })
    }

    pub async fn start(&self) {
        self.sender.send(CloudAgentMessage::Start).await.unwrap();
    }

    pub async fn handle_worker_event(&self, worker_event: WorkerEvent) {
        self.sender
            .send(CloudAgentMessage::HandleWorkerEvent {
                event: worker_event,
            })
            .await
            .unwrap();
    }

    pub async fn perform_run(
        &mut self,
        id: String,
        module_version: ModuleVersion,
        callee: Callee,
        parameters: Vec<Parameter>,
    ) {
        let _ = self
            .sender
            .send(CloudAgentMessage::PerformRun {
                id,
                module_version,
                callee,
                parameters,
            })
            .await;
    }

    pub async fn setup_debugger_listener(
        &mut self,
        id: String,
        sender: oneshot::Sender<StorageValue>,
    ) {
        let _ = self
            .sender
            .send(CloudAgentMessage::SetupDebugListener { id, sender })
            .await;
    }

    pub async fn fulfill_debug_listener(
        &mut self,
        id: String,
        value: StorageValue,
    ) -> Result<(), WorkerError> {
        let _ = self
            .sender
            .send(CloudAgentMessage::FulfillDebugListener { id, value })
            .await;

        Ok(())
    }

    pub async fn enable_debugger(
        &mut self,
        module_id: String,
        module_version: Option<ModuleVersion>,
    ) {
        let _ = self
            .sender
            .send(CloudAgentMessage::EnableDebugger {
                module_id,
                module_version,
            })
            .await;
    }

    pub async fn disable_debugger(&mut self, module_id: String) {
        let _ = self
            .sender
            .send(CloudAgentMessage::DisableDebugger { module_id })
            .await;
    }

    pub async fn reload_configuration(&mut self) {
        let _ = self
            .sender
            .send(CloudAgentMessage::ReloadConfiguration)
            .await;
    }

    pub async fn reload_environment(&mut self) {
        let _ = self.sender.send(CloudAgentMessage::ReloadEnvironment).await;
    }
}

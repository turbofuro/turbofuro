use nanoid::nanoid;
use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::{
    config::{fetch_configuration, ConfigurationCoordinator},
    environment_resolver::SharedEnvironmentResolver,
    errors::WorkerError,
    events::WorkerEvent,
    module_version_resolver::SharedModuleVersionResolver,
    options::CloudOptions,
    shared::{compile_module, get_compiled_module, ModuleVersion, WorkerStatus},
    VERSION,
};
use tokio::sync::mpsc::{self};
use tracing::{debug, error, info, info_span, instrument, warn, Instrument};
use turbofuro_runtime::{
    actor::{Actor, ActorCommand},
    debug::DebugMessage,
    executor::{
        evaluate_parameters, get_timestamp, Callee, DebugState, DebuggerHandle, Environment,
        ExecutionLog, Global, Import, Parameter,
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
    RunFunction {
        id: String,
        module_version: ModuleVersion,
        callee: Callee,
        parameters: Vec<Parameter>,
    },
    EnableDebugger {
        module_id: String,
    },
    DisableDebugger {
        module_id: String,
    },
    ReloadConfiguration,
    ReloadEnvironment,
}

struct CloudAgent {
    // Main agent state
    debug_state: DebugState,
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

    // The child receiver, which we have sender for, never dropped, but used to let operator client pass messages back to the agent
    child_receiver: mpsc::Receiver<CloudAgentMessage>,
    child_sender: mpsc::Sender<CloudAgentMessage>,
}

fn spawn_debugger_handle_reader(
    mut receiver: tokio::sync::mpsc::Receiver<DebugMessage>,
    operator_client: OperatorClientHandle,
) {
    tokio::spawn(async move {
        while let Some(message) = receiver.recv().await {
            operator_client.send_command(message.into()).await;
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
                let _ = self
                    .operator_client
                    .send_command(SendingCommand::UpdateState {
                        version: VERSION,
                        os: std::env::consts::OS,
                        name: self.options.name.clone(),
                        status: self.status.clone(),
                        timestamp: get_timestamp(),
                    })
                    .await;
            }
            CloudAgentMessage::RunFunction {
                id,
                module_version,
                callee,
                parameters,
            } => {
                let (debugger_handle, receiver) = DebuggerHandle::new();
                spawn_debugger_handle_reader(receiver, self.operator_client.clone());

                // TODO: Report error if the run could not be performed ie. could not resolve imported module
                let _ = self
                    .perform_debug_run(id, module_version, callee, parameters, debugger_handle)
                    .await;
            }
            CloudAgentMessage::EnableDebugger { module_id } => {
                let (debugger_handle, receiver) = DebuggerHandle::new();
                spawn_debugger_handle_reader(receiver, self.operator_client.clone());

                if self.debug_state.has_entry(&module_id) {
                    warn!("Debugger already enabled for module {}", module_id.clone());
                    return;
                }

                self.debug_state
                    .add_entry(module_id.clone(), debugger_handle.clone());
                self.global
                    .debug_state
                    .store(Arc::new(self.debug_state.clone()));

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
            CloudAgentMessage::DisableDebugger { module_id } => {
                self.debug_state.remove_entry(&module_id);
                self.global
                    .debug_state
                    .store(Arc::new(self.debug_state.clone()));

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
                let environment_id = {
                    let environment = self.global.environment.read().await;
                    environment.id.clone()
                };

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

                let mut global_environment = self.global.environment.write().await;
                *global_environment = environment;
                info!("Environment updated");
            }
            CloudAgentMessage::Start => self.operator_client.connect().await,
        }
    }

    async fn handle_tick(&mut self) {
        // Let's update state every so often
        self.operator_client
            .send_command(SendingCommand::UpdateState {
                version: VERSION,
                os: std::env::consts::OS,
                name: self.options.name.clone(),
                status: self.status.clone(),
                timestamp: get_timestamp(),
            })
            .await
    }

    #[instrument(level = "info", skip_all)]
    async fn perform_debug_run(
        &mut self,
        id: String,
        module_version: ModuleVersion,
        callee: Callee,
        parameters: Vec<Parameter>,
        debugger: DebuggerHandle,
    ) -> Result<ExecutionLog, WorkerError> {
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

        let global = self.global.clone();
        let environment = { self.global.environment.read().await.clone() };
        let module_version_resolver = self.module_version_resolver.clone();

        // Resolve module version for each import
        let mut imports = HashMap::new();
        for (import_name, import) in &module_version.imports {
            let imported = match import {
                Import::Cloud { id: _, version_id } => {
                    get_compiled_module(version_id, global.clone(), module_version_resolver.clone())
                        .await?
                }
            };
            imports.insert(import_name.to_owned(), imported);
        }

        let mut compiled_module = compile_module(module_version);
        compiled_module.imports = imports;
        let module = Arc::new(compiled_module);
        {
            global.modules.write().await.push(module.clone());
        }

        let mut actor = Actor::new(
            StorageValue::Null(None),
            Arc::new(environment.clone()),
            module.clone(),
            self.global.clone(),
            ActorResources::default(),
            HashMap::new(),
            Some(debugger),
        );

        let (storage, references) =
            evaluate_parameters(&parameters, &ObjectBody::new(), &environment)?;

        actor
            .execute_function(&function_id, storage, references, Some(id))
            .await
            .map_err(WorkerError::from)
    }
}

async fn run_cloud_agent(mut agent: CloudAgent) {
    let mut timer = tokio::time::interval(Duration::from_secs(300));

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
    ) -> Result<Self, WorkerError> {
        let (sender, receiver) = mpsc::channel(16);
        let (child_sender, child_receiver) = mpsc::channel(16);

        let worker_id = nanoid!();
        let operator_url = options.get_operator_url(worker_id.clone())?;

        let operator_client = OperatorClientHandle::new(
            operator_url,
            CloudAgentHandle {
                sender: child_sender.clone(),
            },
        );

        let actor = CloudAgent {
            options,
            environment_resolver,
            module_version_resolver,
            configuration_coordinator,
            global,
            debug_state: DebugState::default(),
            status: WorkerStatus::Starting { warnings: vec![] },
            main_receiver: receiver,
            child_receiver,
            child_sender,
            operator_client,
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
            .send(CloudAgentMessage::RunFunction {
                id,
                module_version,
                callee,
                parameters,
            })
            .await;
    }

    pub async fn enable_debugger(&mut self, module_id: String) {
        let _ = self
            .sender
            .send(CloudAgentMessage::EnableDebugger { module_id })
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

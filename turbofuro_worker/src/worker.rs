use crate::config::{Configuration, WorkerSettings};
use crate::errors::WorkerError;
use crate::events::{WorkerEvent, WorkerEventSender};
use crate::module_version_resolver::SharedModuleVersionResolver;
use crate::options::HttpServerOptions;
use crate::shared::{
    get_compiled_module, resolve_and_install_module, WorkerStoppingReason, WorkerWarning,
};
use crate::utils::shutdown::shutdown_signal;
use crate::utils::sse_receiver_stream::sse_handler;
use axum::body::Body;
use axum::extract::ws::WebSocket;
use axum::extract::{State, WebSocketUpgrade};
use axum::routing::MethodRouter;
use axum::RequestExt;
use axum::{
    http::{self, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use futures_util::{SinkExt, StreamExt, TryFutureExt};
use http::{HeaderValue, Request};
use itertools::Itertools;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::timeout::TimeoutLayer;
use tracing::{debug, error, info, warn};
use turbofuro_runtime::actor::{activate_actor, spawn_ok_or_terminate, Actor, ActorCommand};
use turbofuro_runtime::errors::ExecutionError;
use turbofuro_runtime::executor::{CompiledModule, Environment, Global};
use turbofuro_runtime::http_utils::{build_metadata_from_parts, build_request_object};
use turbofuro_runtime::resources::{
    generate_resource_id, ActorLink, ActorResources, HttpRequestToRespond, HttpResponse,
    OpenWebSocket, PendingHttpRequestBody, Resource, Route, WebSocketCommand,
};
use turbofuro_runtime::{handle_dangling_error, spawn_kv_cleaner, ObjectBody, StorageValue};

async fn handle_websocket_with_errors<'a>(socket: WebSocket, actor: ActorLink) {
    match handle_websocket(socket, actor).await {
        Ok(_) => {}
        Err(e) => {
            error!("Error handling socket: {:?}", e);
        }
    }
}

async fn handle_websocket<'a>(socket: WebSocket, actor_link: ActorLink) -> Result<(), WorkerError> {
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (ws_command_sender, mut ws_command_receiver) = mpsc::channel::<WebSocketCommand>(32);

    // Let's run on connection handler
    {
        let mut actor_resources = ActorResources::default();
        actor_resources.add_websocket(OpenWebSocket(
            generate_resource_id(),
            ws_command_sender.clone(),
        ));

        actor_link
            .send(ActorCommand::TakeResources(actor_resources))
            .await?;

        actor_link
            .send(ActorCommand::Run {
                handler: "onWebSocketConnection".to_owned(),
                storage: ObjectBody::new(),
                references: HashMap::new(),
                sender: None,
                execution_id: None,
            })
            .await?;
    }

    // Start processing of WebSocket command messages
    tokio::spawn(async move {
        while let Some(command) = ws_command_receiver.recv().await {
            match ws_sender.send(command.message).await {
                Ok(_) => {
                    command.responder.send(Ok(())).unwrap();
                }
                Err(e) => {
                    command
                        .responder
                        .send(Err(ExecutionError::StateInvalid {
                            message: "Failed to acknowledge send message to WebSocket".to_owned(),
                            subject: OpenWebSocket::static_type().into(),
                            inner: e.to_string(),
                        }))
                        .unwrap();
                }
            }
        }
    });

    // Process WebSocket messages
    while let Some(msg) = ws_receiver.next().await {
        match msg {
            Ok(msg) => {
                match msg {
                    axum::extract::ws::Message::Close(close_frame) => {
                        let mut initial_storage = HashMap::new();
                        if let Some(close_frame) = close_frame {
                            initial_storage.insert(
                                "reason".to_string(),
                                StorageValue::String(close_frame.reason.to_string()),
                            );
                        } else {
                            initial_storage.insert(
                                "reason".to_string(),
                                StorageValue::String("unknown".to_owned()),
                            );
                        }

                        actor_link
                            .send(ActorCommand::Run {
                                handler: "onWebSocketDisconnection".to_owned(),
                                storage: initial_storage,
                                references: HashMap::new(),
                                sender: None,
                                execution_id: None,
                            })
                            .await?;
                    }
                    msg => {
                        let message: StorageValue = match msg {
                            axum::extract::ws::Message::Text(text) => {
                                StorageValue::String(text.to_string())
                            }
                            axum::extract::ws::Message::Binary(bytes) => StorageValue::Array(
                                bytes
                                    .iter()
                                    .map(|b| StorageValue::Number(*b as f64))
                                    .collect(),
                            ),
                            axum::extract::ws::Message::Ping(_) => {
                                StorageValue::String("ping".to_string())
                            }
                            axum::extract::ws::Message::Pong(_) => {
                                StorageValue::String("pong".to_string())
                            }
                            _ => StorageValue::Null(None),
                        };

                        let mut initial_storage = HashMap::new();
                        initial_storage.insert("message".to_string(), message);

                        actor_link
                            .send(ActorCommand::Run {
                                handler: "onWebSocketMessage".to_owned(),
                                storage: initial_storage,
                                references: HashMap::new(),
                                sender: None,
                                execution_id: None,
                            })
                            .await?;
                    }
                };
            }
            Err(error) => {
                let mut initial_storage = ObjectBody::new();
                initial_storage.insert(
                    "reason".to_string(),
                    StorageValue::String("errored".to_owned()),
                );
                initial_storage
                    .insert("error".to_string(), StorageValue::String(error.to_string()));

                actor_link
                    .send(ActorCommand::Run {
                        handler: "onWebSocketDisconnection".to_owned(),
                        storage: initial_storage,
                        references: HashMap::new(),
                        sender: None,
                        execution_id: None,
                    })
                    .await?;
            }
        }
    }

    // After WebSocket is closed, we need to terminate the actor
    actor_link.send(ActorCommand::Terminate).await?;

    Ok(())
}

async fn handle_request(
    State(global): State<Arc<Global>>,
    mut request: Request<Body>,
    route: Route,
) -> Result<Response, WorkerError> {
    let module = get_compiled_module(route.module_version_id.as_str(), global.clone()).await?;

    let ws = request.extract_parts::<WebSocketUpgrade>().await.ok();

    let (http_response_sender, http_response_receiver) = oneshot::channel::<HttpResponse>();
    let mut resources = ActorResources::default();
    resources.add_http_request_to_respond(HttpRequestToRespond {
        id: generate_resource_id(),
        response_sender: http_response_sender,
    });

    // Build initial storage
    let mut initial_storage = ObjectBody::new();
    if route.parse_body {
        let (request_object, mut request_resources) = build_request_object(request).await;
        resources.append(&mut request_resources);
        initial_storage.insert("request".to_string(), StorageValue::Object(request_object));
    } else {
        let (mut parts, body) = request.into_parts();
        let (request_object, _) = build_metadata_from_parts(&mut parts).await;
        initial_storage.insert("request".to_string(), StorageValue::Object(request_object));
        resources.add_pending_request_body(PendingHttpRequestBody(generate_resource_id(), body));
    }

    let handlers = route.handlers.clone();

    let debugger = global.debug_state.load().get_debugger(&module.module_id);

    let actor = Actor::new(
        StorageValue::Null(None),
        global.environment.load().clone(),
        module,
        global.clone(),
        resources,
        handlers,
        debugger,
    );

    let actor_id = actor.get_id().to_owned();
    let actor_link = activate_actor(actor);
    global
        .registry
        .actors
        .insert(actor_id.clone(), actor_link.clone());

    let (response_sender, response_receiver) =
        oneshot::channel::<Result<StorageValue, ExecutionError>>();

    actor_link
        .send(ActorCommand::Run {
            handler: "onHttpRequest".to_owned(),
            storage: initial_storage,
            references: HashMap::new(),
            sender: Some(response_sender),
            execution_id: None,
        })
        .await?;

    spawn_ok_or_terminate(actor_link.clone(), response_receiver);

    match http_response_receiver.await {
        Ok(wrapper) => match wrapper {
            HttpResponse::Normal(response, responder) => {
                responder.send(Ok(())).unwrap();
                actor_link.send(ActorCommand::Terminate).await?;
                Ok(response)
            }
            HttpResponse::WebSocket(responder) => {
                let ws = match ws {
                    Some(ws) => {
                        responder.send(Ok(())).unwrap();
                        ws
                    }
                    None => {
                        responder
                            .send(Err(ExecutionError::StateInvalid {
                                message: "Could not open WebSocket on non-upgrade HTTP request"
                                    .into(),
                                subject: "http_server".into(),
                                inner: "No upgrade headers".into(),
                            }))
                            .unwrap();
                        return Ok(StatusCode::INTERNAL_SERVER_ERROR.into_response());
                    }
                };

                Ok(ws.on_upgrade(move |socket| handle_websocket_with_errors(socket, actor_link)))
            }
            HttpResponse::ServerSentEvents {
                event_receiver,
                keep_alive,
                disconnect_sender,
                responder,
            } => {
                responder.send(Ok(())).unwrap();
                Ok(sse_handler(event_receiver, keep_alive, disconnect_sender).into_response())
            }
        },
        Err(_) => Ok(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
}

pub struct WorkerHttpServer {
    routes: Vec<Route>,
}

impl WorkerHttpServer {
    pub fn new(routes: Vec<Route>) -> Self {
        Self { routes }
    }

    pub fn materialize(
        &self,
        global: Arc<Global>,
        settings: WorkerSettings,
    ) -> Result<Router, WorkerError> {
        let mut router = Router::new();

        let grouped_http_endpoints = self
            .routes
            .clone()
            .into_iter()
            .chunk_by(|endpoint| endpoint.path.clone())
            .into_iter()
            .map(|(path, route_handlers)| (path, route_handlers.collect_vec()))
            .collect::<Vec<(String, Vec<Route>)>>();

        for (path, route_handlers) in grouped_http_endpoints {
            let mut method_router = MethodRouter::new();
            for route in route_handlers {
                let route_cloned = route.clone();
                info!("Registering route {} {}", route.method.to_uppercase(), path);
                match route.method.to_lowercase().as_str() {
                    "get" => {
                        method_router = method_router
                            .get(move |state, req| handle_request(state, req, route_cloned))
                    }
                    "post" => {
                        method_router = method_router
                            .post(move |state, req| handle_request(state, req, route_cloned))
                    }
                    "put" => {
                        method_router = method_router
                            .put(move |state, req| handle_request(state, req, route_cloned))
                    }
                    "patch" => {
                        method_router = method_router
                            .patch(move |state, req| handle_request(state, req, route_cloned))
                    }
                    "delete" => {
                        method_router = method_router
                            .delete(move |state, req| handle_request(state, req, route_cloned))
                    }
                    "head" => {
                        method_router = method_router
                            .head(move |state, req| handle_request(state, req, route_cloned))
                    }
                    "options" => {
                        method_router = method_router.options(move |d: State<Arc<Global>>, r| {
                            handle_request(d, r, route_cloned)
                        })
                    }
                    _ => {
                        panic!("Unsupported method: {}", route.method)
                    }
                };
            }
            router = router.route(&path, method_router);
        }

        match settings.timeout {
            crate::config::Timeout::Disabled => {
                // No-op
            }
            crate::config::Timeout::Default => {
                router = router.layer(TimeoutLayer::new(std::time::Duration::from_secs(60)));
            }
            crate::config::Timeout::Custom { seconds } => {
                router = router.layer(TimeoutLayer::new(std::time::Duration::from_secs(seconds)));
            }
        }

        match settings.cors {
            crate::config::Cors::Disabled => {
                // No-op
            }
            crate::config::Cors::Any => {
                router = router.layer(CorsLayer::permissive());
            }
            crate::config::Cors::Origins {
                origins: raw_origins,
            } => {
                // Map String to HeaderValue
                let mut origins: Vec<HeaderValue> = Vec::with_capacity(raw_origins.len());
                for raw_origin in raw_origins {
                    let header_value = HeaderValue::from_str(&raw_origin).map_err(|e| {
                        WorkerError::MalformedConfiguration {
                            message: format!("Could not parse origin: {}", e),
                        }
                    })?;
                    origins.push(header_value);
                }

                router = router.layer(
                    CorsLayer::new()
                        .allow_headers(Any)
                        .allow_methods(Any)
                        .allow_origin(origins)
                        .expose_headers(Any),
                );
            }
            crate::config::Cors::AnyWithCredentials => {
                router = router.layer(CorsLayer::very_permissive());
            }
        }

        match settings.compression {
            crate::config::Compression::Disabled => {
                // No-op
            }
            crate::config::Compression::Automatic => router = router.layer(CompressionLayer::new()),
        }

        Ok(router.with_state(global))
    }
}

#[derive(Debug)]
struct ModuleState {
    module_id: String,
    manager_id: String,
    module: Arc<CompiledModule>,
}

pub struct Worker {
    config: Configuration,
    global: Arc<Global>,
    module_version_resolver: SharedModuleVersionResolver,
    module_managers: Vec<ModuleState>,

    // To be send by worker to shutdown axum app
    router_shutdown: Option<oneshot::Sender<bool>>,

    // To be received by worker to wait for axum app to shutdown
    router_shutdown_completed: Option<oneshot::Receiver<()>>,

    // Sender to propagate worker events
    event_sender: WorkerEventSender,
}

impl Worker {
    pub fn new(
        config: Configuration,
        global: Arc<Global>,
        module_version_resolver: SharedModuleVersionResolver,
        event_sender: WorkerEventSender,
    ) -> Self {
        Self {
            config,
            global,
            module_version_resolver,
            router_shutdown: None,
            router_shutdown_completed: None,
            event_sender,
            module_managers: vec![],
        }
    }

    pub async fn start(&mut self) -> Result<Router, WorkerError> {
        let debug_state = self.global.debug_state.load();

        // Spawn KV cleaner
        spawn_kv_cleaner();

        let modules = {
            futures_util::stream::iter(self.config.modules.iter().map(|m| {
                let debug_entry = debug_state.get_entry(m.module_id.as_str());

                get_or_resolve_and_install(
                    &m.module_version_id,
                    self.global.clone(),
                    self.module_version_resolver.clone(),
                    debug_entry.and_then(|e| e.module.clone()),
                )
                .map_err(|e| (e, m.module_id.clone(), m.module_version_id.clone()))
            }))
            .buffer_unordered(16)
            .collect::<Vec<Result<Arc<CompiledModule>, (WorkerError, String, String)>>>()
            .await
        };

        let mut modules_to_start: Vec<Arc<CompiledModule>> = vec![];
        for module in modules {
            match module {
                Ok(module) => {
                    modules_to_start.push(module);
                }
                Err((err, module_id, module_version_id)) => {
                    error!(
                        "Could not get compiled module for module id: {} version id: {}: {:?}",
                        module_id, module_version_id, err,
                    );
                    self.event_sender
                        .send(WorkerEvent::WarningRaised(
                            WorkerWarning::ModuleCouldNotBeLoaded {
                                module_id: module_id.clone(),
                                module_version_id: module_version_id.clone(),
                                error: err,
                            },
                        ))
                        .await
                }
            }
        }

        // Include modules that are not in the configuration, but are in the debug state
        for debug_entry in debug_state.entries.iter() {
            if !self
                .config
                .modules
                .iter()
                .any(|m| m.module_id == debug_entry.module_id)
            {
                if let Some(module) = debug_entry.module.clone() {
                    modules_to_start.push(module);
                }
            }
        }

        let environment: Arc<Environment> = self.global.environment.load().clone();

        // We would prepare a HTTP server, but the default value is OK for now
        // TODO: Disable HTTP server when no routes are defined

        let mut waits = vec![];
        for module in modules_to_start {
            // Make a copy of the sender (+ module id) so we can send warnings later
            let module_version_id = module.id.clone();
            let event_sender = self.event_sender.clone();
            let debugger = debug_state.get_debugger(&module.module_id);

            let actor = Actor::new(
                StorageValue::Null(None),
                environment.clone(),
                module.clone(),
                self.global.clone(),
                ActorResources::default(),
                HashMap::new(),
                debugger,
            );
            let actor_id = actor.get_id().to_owned();
            let actor_link = activate_actor(actor);

            self.module_managers.push(ModuleState {
                module_id: module.id.clone(),
                manager_id: actor_id.clone(),
                module: module.clone(),
            });

            self.global
                .registry
                .actors
                .insert(actor_id, actor_link.clone());

            for starter in &module.module_starters {
                let (run_sender, run_receiver) = tokio::sync::oneshot::channel();
                let _ = actor_link
                    .send(ActorCommand::RunFunctionRef {
                        function_ref: starter.get_id().to_owned(),
                        storage: HashMap::new(),
                        references: HashMap::new(),
                        sender: Some(run_sender),
                        execution_id: None,
                    })
                    .await;

                let event_sender = event_sender.clone();
                let module_version_id = module_version_id.clone();
                waits.push(async move {
                    let response = run_receiver.await;
                    match response {
                        Ok(result) => {
                            match result {
                                Ok(_) => {
                                    // All went well, do nothing
                                }
                                Err(err) => {
                                    event_sender
                                        .send(WorkerEvent::WarningRaised(
                                            WorkerWarning::ModuleStartupFailed {
                                                module_id: module_version_id,
                                                error: WorkerError::from(err),
                                            },
                                        ))
                                        .await;
                                }
                            }
                        }
                        Err(_) => {
                            event_sender
                                .send(WorkerEvent::WarningRaised(
                                    WorkerWarning::ModuleStartupFailed {
                                        module_id: module_version_id,
                                        error: WorkerError::from(
                                            ExecutionError::new_missing_response_from_actor(),
                                        ),
                                    },
                                ))
                                .await;
                        }
                    }
                });
            }
        }

        futures_util::future::join_all(waits).await;

        let mut routes = self.global.registry.router.lock().await;
        let http_server = WorkerHttpServer::new(routes.get_routes().clone());
        routes.clear();

        let router = http_server.materialize(self.global.clone(), self.config.settings.clone())?;

        Ok(router)
    }

    pub async fn start_with_http_server(
        &mut self,
        http_server_options: HttpServerOptions,
    ) -> Result<(), WorkerError> {
        self.event_sender.send(WorkerEvent::WorkerStarting).await;

        let router = self.start().await?;

        let (shutdown_sender, shutdown_receiver) = oneshot::channel::<bool>();
        self.router_shutdown = Some(shutdown_sender);

        let (shutdown_completed_sender, shutdown_completed_receiver) = oneshot::channel::<()>();
        self.router_shutdown_completed = Some(shutdown_completed_receiver);

        // The HTTP server starts in a separate task, if it fails to start the worker is not stopped
        // Perhaps this behavior should be changed in the future
        let event_sender = self.event_sender.clone();
        tokio::spawn(async move {
            let addr = match http_server_options.get_socket_addr() {
                Ok(addr) => addr,
                Err(err) => {
                    error!(
                        "Could not get a valid socket address for HTTP server: {}",
                        err
                    );
                    event_sender
                        .send(WorkerEvent::WarningRaised(
                            WorkerWarning::HttpServerFailedToStart {
                                message: err.to_string(),
                            },
                        ))
                        .await;

                    return;
                }
            };

            info!("Starting HTTP server on {}", addr);

            let handle = axum_server::Handle::new();

            // Start shutdown listener
            let listener_handle = handle.clone();
            tokio::spawn(async move {
                match shutdown_receiver.await {
                    Ok(force_quit) => {
                        if force_quit {
                            listener_handle.shutdown()
                        } else {
                            listener_handle.graceful_shutdown(Some(Duration::from_secs(15)));
                        }
                    }
                    Err(err) => {
                        error!("Error receiving shutdown signal: {:?}", err);
                    }
                }
            });

            // Start HTTP server
            match axum_server::bind(addr)
                .handle(handle)
                .serve(router.into_make_service())
                .await
            {
                Ok(_) => {
                    // The server exited normally
                }
                Err(err) => {
                    error!("HTTP server startup error: {:?}", err);
                    event_sender
                        .send(WorkerEvent::WarningRaised(
                            WorkerWarning::HttpServerFailedToStart {
                                message: err.to_string(),
                            },
                        ))
                        .await;

                    return;
                }
            }

            shutdown_completed_sender.send(()).unwrap();
        });

        self.event_sender.send(WorkerEvent::WorkerStarted).await;

        Ok(())
    }

    pub async fn stop(&mut self, reason: WorkerStoppingReason) {
        self.event_sender
            .send(WorkerEvent::WorkerStopping(reason.clone()))
            .await;

        // For each managed module, call stopper if it exists
        for module in self.module_managers.iter() {
            match self.global.registry.actors.get(&module.manager_id) {
                Some(manager) => {
                    for stopper in &module.module.module_stoppers {
                        let _ = manager
                            .send(ActorCommand::RunFunctionRef {
                                function_ref: stopper.get_id().to_owned(),
                                storage: HashMap::new(),
                                references: HashMap::new(),
                                sender: None,
                                execution_id: None,
                            })
                            .await;
                    }
                }
                None => {
                    warn!(
                        "Could not find module manager for module {}",
                        module.module_id
                    );
                }
            }
        }
        self.module_managers.clear();

        if let Some(shutdown) = self.router_shutdown.take() {
            debug!("Sending shutdown to HTTP server");

            let force_quit = matches!(reason, WorkerStoppingReason::RequestedByCloudAgent);
            handle_dangling_error!(shutdown.send(force_quit));
        }

        debug!("Propagating terminate to actors");
        for pair in self.global.registry.actors.iter_mut() {
            let _ = pair.send(ActorCommand::Terminate).await;
        }

        if let Some(shutdown_completed) = self.router_shutdown_completed.take() {
            tokio::select! {
                _ = shutdown_completed => {
                    debug!("HTTP server shutdown");
                }
                _ = shutdown_signal() => {
                    info!("Force quit signal received. Terminating immediately without waiting for HTTP server to shutdown");
                }
            }
        }

        self.event_sender
            .send(WorkerEvent::WorkerStopped(reason))
            .await
    }
}

// Just a helper function to avoid type annotations in the resolver loop
async fn get_or_resolve_and_install(
    id: &str,
    global: Arc<Global>,
    module_version_resolver: SharedModuleVersionResolver,
    option: Option<Arc<CompiledModule>>,
) -> Result<Arc<CompiledModule>, WorkerError> {
    match option {
        Some(module) => Ok(module),
        None => resolve_and_install_module(id, global, module_version_resolver).await,
    }
}

#[cfg(test)]
mod test_worker {}

extern crate log;

use crate::config::{Configuration, WorkerSettings};
use crate::environment_resolver::SharedEnvironmentResolver;
use crate::module_version_resolver::SharedModuleVersionResolver;
use crate::HttpServerOptions;
use async_recursion::async_recursion;
use axum::body::Body;
use axum::extract::ws::WebSocket;
use axum::extract::{State, WebSocketUpgrade};
use axum::routing::MethodRouter;
use axum::{
    http::{self, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use axum::{Json, RequestExt};
use futures_util::{SinkExt, StreamExt};
use http::{HeaderValue, Request};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc::Sender;
use tokio::sync::{mpsc, oneshot};
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::timeout::TimeoutLayer;
use turbofuro_runtime::actor::{activate_actor, Actor, ActorCommand};
use turbofuro_runtime::errors::ExecutionError;
use turbofuro_runtime::executor::{
    CompiledModule, Environment, Function, Global, Import, Step, Steps,
};
use turbofuro_runtime::http_utils::{build_metadata_from_parts, build_request_object};
use turbofuro_runtime::resources::{
    ActorLink, ActorResources, HttpRequestToRespond, HttpResponse, OpenWebSocket,
    PendingHttpRequestBody, Resource, Route, WebSocketCommand,
};
use turbofuro_runtime::{ObjectBody, StorageValue};

use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, info_span, instrument, Instrument};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", tag = "type")]
pub enum ModuleVersionType {
    Http,
    WebSocket,
    Actor,
    Library,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModuleVersion {
    pub id: String,
    #[serde(rename = "moduleId")]
    pub module_id: String,
    pub instructions: Steps,
    pub handlers: HashMap<String, String>,
    pub imports: HashMap<String, Import>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "app_error", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AppError {
    ModuleVersionNotFound,
    MalformedModuleVersion,
    EnvironmentNotFound,
    MalformedEnvironment,
    ExecutionFailed {
        #[serde(flatten)]
        error: ExecutionError,
    },
}

impl From<ExecutionError> for AppError {
    fn from(error: ExecutionError) -> Self {
        AppError::ExecutionFailed { error }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            AppError::ModuleVersionNotFound => (StatusCode::NOT_FOUND, "Module version not found"),
            AppError::MalformedModuleVersion => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Malformed module version",
            ),
            AppError::EnvironmentNotFound => (StatusCode::NOT_FOUND, "Environment not found"),
            AppError::MalformedEnvironment => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Malformed module version",
            ),
            AppError::ExecutionFailed { .. } => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Failed to run module")
            }
        };

        let body = Json(json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}

#[derive(Debug)]
pub enum WorkerError {
    IncorrectParameters(String),
    IncorrectEnvironmentVariable(String, String),
    ConfigurationFetch,
}

impl Display for WorkerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkerError::IncorrectParameters(message) => {
                write!(f, "Incorrect parameters: {}", message)
            }
            WorkerError::ConfigurationFetch => {
                write!(f, "Could not fetch configuration")
            }
            WorkerError::IncorrectEnvironmentVariable(key, message) => {
                write!(f, "Incorrect environment variable {}: {}", key, message)
            }
        }
    }
}

async fn handle_socket_with_errors<'a>(socket: WebSocket, actor: Sender<ActorCommand>) {
    match handle_socket(socket, actor).await {
        Ok(_) => {}
        Err(e) => {
            error!("Error handling socket: {:?}", e);
        }
    }
}

async fn handle_socket<'a>(socket: WebSocket, actor: Sender<ActorCommand>) -> Result<(), AppError> {
    let (mut sender, mut receiver) = socket.split();
    let (websocket_sender, mut websocket_receiver) = mpsc::channel::<WebSocketCommand>(32);
    {
        let initial_storage = HashMap::new();
        let mut scoped_resources = ActorResources::default();
        scoped_resources
            .websockets
            .push(OpenWebSocket(websocket_sender.clone()));
        actor
            .send(ActorCommand::TakeResource {
                resources: scoped_resources,
            })
            .await
            .map_err(|_| {
                AppError::from(ExecutionError::ActorCommandFailed {
                    message: "Could not send take resource to actor".to_owned(),
                })
            })?;

        actor
            .send(ActorCommand::Run {
                handler: "onWebSocketConnection".to_owned(),
                storage: initial_storage,
                sender: None,
            })
            .await
            .map_err(|_| {
                AppError::from(ExecutionError::ActorCommandFailed {
                    message: "Could not send run command (handler: onWebSocketConnection) to actor"
                        .to_owned(),
                })
            })?;
    }

    tokio::spawn(async move {
        while let Some(command) = websocket_receiver.recv().await {
            match sender.send(command.message).await {
                Ok(_) => {
                    command.responder.send(Ok(())).unwrap();
                }
                Err(e) => {
                    command
                        .responder
                        .send(Err(ExecutionError::StateInvalid {
                            message: "Failed to acknowledge send message to WebSocket".to_owned(),
                            subject: OpenWebSocket::get_type().into(),
                            inner: e.to_string(),
                        }))
                        .unwrap();
                }
            }
        }
    });

    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(msg) => {
                match msg {
                    axum::extract::ws::Message::Close(_) => {
                        let mut initial_storage = HashMap::new();
                        initial_storage.insert(
                            "reason".to_string(),
                            StorageValue::String("leaving".to_owned()),
                        );

                        actor
                            .send(ActorCommand::Run {
                                handler: "onWebSocketDisconnection".to_owned(),
                                storage: initial_storage,
                                sender: None
                            })
                            .await
                            .map_err(|_| {
                                AppError::from(ExecutionError::ActorCommandFailed {
                                    message: "Could not send run command (handler: onWebSocketDisconnection) to actor"
                                        .to_owned(),
                                })
                            })?;
                    }
                    msg => {
                        let message: StorageValue = match msg {
                            axum::extract::ws::Message::Text(text) => StorageValue::String(text),
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

                        actor
                            .send(ActorCommand::Run {
                                handler: "onWebSocketMessage".to_owned(),
                                storage: initial_storage,
                                sender: None
                            })
                            .await
                            .map_err(|_| {
                                AppError::from(ExecutionError::ActorCommandFailed {
                                    message: "Could not send run command (handler: onWebSocketMessage) to actor"
                                        .to_owned(),
                                })
                            })?;
                    }
                };
            }
            Err(error) => {
                let mut initial_storage = HashMap::new();
                initial_storage.insert(
                    "reason".to_string(),
                    StorageValue::String("errored".to_owned()),
                );
                initial_storage
                    .insert("error".to_string(), StorageValue::String(error.to_string()));

                actor
                    .send(ActorCommand::Run {
                        handler: "onWebSocketDisconnection".to_owned(),
                        storage: initial_storage,
                        sender: None,
                    })
                    .await
                    .map_err(|_| {
                        AppError::from(ExecutionError::ActorCommandFailed {
                            message: "Could not send run command (handler: onWebSocketDisconnection) to actor"
                                .to_owned(),
                        })
                    })?;
            }
        }
    }

    actor.send(ActorCommand::Terminate).await.map_err(|_| {
        AppError::from(ExecutionError::ActorCommandFailed {
            message: "Could not send terminate command to actor".to_owned(),
        })
    })?;

    Ok(())
}

pub fn compile_module(module_version: ModuleVersion) -> CompiledModule {
    let local_functions: Vec<Function> = module_version
        .instructions
        .iter()
        .filter(|s| {
            matches!(
                s,
                Step::DefineFunction {
                    exported: false,
                    ..
                } | Step::DefineNativeFunction {
                    exported: false,
                    ..
                }
            )
        })
        .map(|s| match s {
            Step::DefineFunction {
                id,
                parameters: _,
                body,
                exported: _,
            } => Function::Normal {
                id: id.clone(),
                body: body.clone(),
            },
            Step::DefineNativeFunction {
                id,
                native_id,
                parameters: _,
                exported: _,
            } => Function::Native {
                id: id.to_owned(),
                native_id: native_id.to_owned(),
            },
            other => {
                panic!("Expected function definition, got step: {:?}", other)
            }
        })
        .collect_vec();

    let exported_functions: Vec<Function> = module_version
        .instructions
        .iter()
        .filter(|s| {
            matches!(
                s,
                Step::DefineFunction { exported: true, .. }
                    | Step::DefineNativeFunction { exported: true, .. }
            )
        })
        .map(|s| match s {
            Step::DefineFunction {
                id,
                parameters: _,
                body,
                exported: _,
            } => Function::Normal {
                id: id.clone(),
                body: body.clone(),
            },
            Step::DefineNativeFunction {
                id,
                native_id,
                parameters: _,
                exported: _,
            } => Function::Native {
                id: id.to_owned(),
                native_id: native_id.to_owned(),
            },
            other => {
                panic!("Expected function definition, got step: {:?}", other)
            }
        })
        .collect_vec();

    CompiledModule {
        id: module_version.id,
        module_id: module_version.module_id,
        local_functions,
        exported_functions,
        handlers: module_version.handlers,
        imports: HashMap::new(),
    }
}

#[async_recursion]
#[instrument(skip(global, module_version_resolver), level = "debug")]
pub async fn get_compiled_module(
    id: &str,
    global: Arc<Global>,
    module_version_resolver: SharedModuleVersionResolver,
) -> Result<Arc<CompiledModule>, AppError> {
    let cached_module = {
        global
            .modules
            .read()
            .await
            .iter()
            .find(|m| m.id == id)
            .cloned()
    };

    if let Some(cached) = cached_module {
        debug!("Returning cached compiled module: {}", id);
        return Ok(cached);
    }

    debug!("Fetching module: {}", id);
    let module_version: ModuleVersion = {
        module_version_resolver
            .get_module_version(id)
            .instrument(info_span!("get_module_version"))
            .await?
    };

    // Resolve module version for each import
    let mut imports = HashMap::new();
    for (import_name, import) in &module_version.imports {
        debug!("Fetching import: {}", import_name);
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
    Ok(module)
}

async fn handle_request(
    State(app_state): State<AppState>,
    mut request: Request<Body>,
    route: Route,
) -> Result<Response, AppError> {
    let module = get_compiled_module(
        route.module_version_id.as_str(),
        app_state.global.clone(),
        app_state.module_version_resolver,
    )
    .await?;

    let ws = request.extract_parts::<WebSocketUpgrade>().await.ok();

    let (http_response_sender, http_response_receiver) = oneshot::channel::<HttpResponse>();
    let mut scoped_resources = ActorResources::default();
    scoped_resources
        .http_requests_to_respond
        .push(HttpRequestToRespond(http_response_sender));

    // Build initial storage
    let mut initial_storage = ObjectBody::new();

    if route.parse_body {
        let request_object = build_request_object(request).await;
        initial_storage.insert("request".to_string(), StorageValue::Object(request_object));
    } else {
        let (mut parts, body) = request.into_parts();
        let (request_object, _) = build_metadata_from_parts(&mut parts).await;
        initial_storage.insert("request".to_string(), StorageValue::Object(request_object));

        scoped_resources
            .pending_request_body
            .push(PendingHttpRequestBody(body));
    }

    let handlers = route.handlers.clone();

    let actor = Actor::new(
        StorageValue::Null(None),
        app_state.environment.clone(),
        module,
        app_state.global.clone(),
        scoped_resources,
        handlers,
    );

    let actor_id = actor.get_id().to_owned();
    let sender = activate_actor(actor);
    app_state
        .global
        .registry
        .actors
        .insert(actor_id.clone(), ActorLink(sender.clone()));

    let (response_sender, response_receiver) =
        oneshot::channel::<Result<StorageValue, ExecutionError>>();
    sender
        .send(ActorCommand::Run {
            handler: "onRequest".to_owned(),
            storage: initial_storage,
            sender: Some(response_sender),
        })
        .await
        .map_err(|_| {
            AppError::from(ExecutionError::ActorCommandFailed {
                message: "Could not send run command (handler: onRequest) to actor".to_owned(),
            })
        })?;

    let killer_sender = sender.clone();
    tokio::spawn(async move {
        match response_receiver.await {
            Ok(resp) => {
                match resp {
                    Ok(_) => {
                        // Do nothing
                    }
                    Err(e) => {
                        // If the execution has failed we should terminate the actor
                        match killer_sender
                            .send(ActorCommand::Terminate)
                            .await
                            .map_err(|_| {
                                AppError::from(ExecutionError::ActorCommandFailed {
                                    message: "Could not send terminate command to actor".to_owned(),
                                })
                            }) {
                            Ok(_) => {}
                            Err(_) => {
                                error!("Could not send terminate command to actor");
                            }
                        }
                        error!("Error running onRequest: {:?} sending error response", e);
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

    match http_response_receiver.await {
        Ok(wrapper) => {
            let response = match wrapper {
                HttpResponse::Normal(response, responder) => {
                    responder.send(Ok(())).unwrap();
                    sender.send(ActorCommand::Terminate).await.map_err(|_| {
                        AppError::from(ExecutionError::ActorCommandFailed {
                            message: "Could not send terminate command to actor".to_owned(),
                        })
                    })?;
                    response
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

                    ws.on_upgrade(move |socket| handle_socket_with_errors(socket, sender))
                }
            };

            Ok(response)
        }
        Err(_) => Ok(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
}

#[derive(Clone)]
struct AppState {
    module_version_resolver: SharedModuleVersionResolver,
    environment: Arc<Environment>,
    global: Arc<Global>,
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
        module_version_resolver: SharedModuleVersionResolver,
        environment: Arc<Environment>,
        settings: WorkerSettings,
    ) -> Result<Router, WorkerError> {
        let mut router = Router::new();

        let grouped_http_endpoints = self
            .routes
            .clone()
            .into_iter()
            .group_by(|endpoint| endpoint.path.clone())
            .into_iter()
            .map(|(path, route_handlers)| (path, route_handlers.collect_vec()))
            .collect::<Vec<(String, Vec<Route>)>>();

        for (path, route_handlers) in grouped_http_endpoints {
            let mut method_router = MethodRouter::new();
            for route in route_handlers {
                let route_cloned = route.clone();
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
                        method_router = method_router.options(move |d: State<AppState>, r| {
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
                        WorkerError::IncorrectParameters(format!("Could not parse origin: {}", e))
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

        Ok(router.with_state(AppState {
            module_version_resolver,
            global,
            environment,
        }))
    }
}

pub struct Worker {
    config: Configuration,
    global: Arc<Global>,
    module_version_resolver: SharedModuleVersionResolver,
    environment_resolver: SharedEnvironmentResolver,

    // To be send by worker to shutdown axum app
    router_shutdown: Option<oneshot::Sender<()>>,

    // To be received by worker to wait for axum app to shutdown
    router_shutdown_completed: Option<oneshot::Receiver<()>>,
}

impl Worker {
    pub fn new(
        config: Configuration,
        global: Arc<Global>,
        module_version_resolver: SharedModuleVersionResolver,
        environment_resolver: SharedEnvironmentResolver,
    ) -> Self {
        Self {
            config,
            global,
            module_version_resolver,
            environment_resolver,
            router_shutdown: None,
            router_shutdown_completed: None,
        }
    }

    pub async fn start(&mut self) -> Result<Router, WorkerError> {
        let environment: Environment = match &self.config.environment_id {
            Some(id) => {
                // Let's fetch environment
                match self
                    .environment_resolver
                    .lock()
                    .await
                    .get_environment(id)
                    .instrument(info_span!("get_environment"))
                    .await
                {
                    Ok(environment) => environment,
                    Err(e) => {
                        error!("Could not fetch environment: {:?}", e);
                        Environment::new("empty".into())
                    }
                }
            }
            None => {
                info!("No environment specified in the config, using empty environment");
                Environment::new("empty".into())
            }
        };

        // Update global environment
        {
            let mut e = self.global.environment.write().await;
            *e = environment.clone()
        }
        let environment = Arc::new(environment);

        let modules = {
            futures_util::stream::iter(self.config.modules.iter().map(|m| {
                get_compiled_module(
                    &m.module_version_id,
                    self.global.clone(),
                    self.module_version_resolver.clone(),
                )
            }))
            .buffer_unordered(16)
            .collect::<Vec<Result<Arc<CompiledModule>, AppError>>>()
            .await
        };

        let mut modules_to_start: Vec<Arc<CompiledModule>> = vec![];
        for module in modules {
            match module {
                Ok(module) => {
                    modules_to_start.push(module);
                }
                Err(e) => {
                    error!("Could not get compiled module: {:?}", e);
                }
            }
        }

        // We would prepare a HTTP server, but the default value is OK for now
        // TODO: Disable HTTP server when no routes are defined

        let mut waits = vec![];
        for module in modules_to_start {
            let actor = Actor::new_module_initiator(
                StorageValue::Null(None),
                environment.clone(),
                module,
                self.global.clone(),
                ActorResources::default(), // TODO: Add router here?
            );
            let actor_id = actor.get_id().to_owned();
            let sender = activate_actor(actor);

            self.global
                .registry
                .actors
                .insert(actor_id, ActorLink(sender.clone()));

            let (run_sender, run_receiver) = tokio::sync::oneshot::channel();

            let _ = sender
                .send(ActorCommand::Run {
                    handler: "onStart".to_owned(),
                    storage: HashMap::new(),
                    sender: Some(run_sender),
                })
                .await
                .map_err(|_| {
                    AppError::from(ExecutionError::ActorCommandFailed {
                        message: "Could not send run command (handler: onRequest) to actor"
                            .to_owned(),
                    })
                }); // TODO: Handle error

            waits.push(run_receiver);
        }

        futures_util::future::join_all(waits).await;

        let mut routes = self.global.registry.router.lock().await;
        let http_server = WorkerHttpServer::new(routes.get_routes().clone());
        routes.clear();

        let router = http_server.materialize(
            self.global.clone(),
            self.module_version_resolver.clone(),
            environment.clone(),
            self.config.settings.clone(),
        )?;

        Ok(router)
    }

    pub async fn start_with_http_server(
        &mut self,
        http_server_options: HttpServerOptions,
    ) -> Result<(), WorkerError> {
        let router = self.start().await?;

        let (shutdown_sender, shutdown_receiver) = tokio::sync::oneshot::channel::<()>();
        self.router_shutdown = Some(shutdown_sender);

        let (shutdown_completed_sender, shutdown_completed_receiver) =
            tokio::sync::oneshot::channel::<()>();
        self.router_shutdown_completed = Some(shutdown_completed_receiver);

        tokio::spawn(async move {
            let addr = ([0, 0, 0, 0], http_server_options.port).into();
            info!("Starting HTTP server on {}", addr);

            let handle = axum_server::Handle::new();

            // Start shutdown listener
            let listener_handle = handle.clone();
            tokio::spawn(async move {
                match shutdown_receiver.await {
                    Ok(_) => {
                        listener_handle.graceful_shutdown(Some(Duration::from_secs(15)));
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
                Ok(_) => {}
                Err(e) => {
                    error!("HTTP server startup error: {:?}", e);
                }
            }

            shutdown_completed_sender.send(()).unwrap();
        });

        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(shutdown) = self.router_shutdown.take() {
            debug!("Sending shutdown to HTTP server");
            shutdown.send(()).unwrap();
        }

        debug!("Propagating terminate to actors");
        for pair in self.global.registry.actors.iter_mut() {
            match pair.0.send(ActorCommand::Terminate).await {
                Ok(_) => {}
                Err(_e) => {
                    // TODO: Send errors
                }
            }
        }

        if let Some(shutdown_completed) = self.router_shutdown_completed.take() {
            debug!("Waiting for HTTP server to shutdown");
            shutdown_completed.await.unwrap();
            debug!("HTTP server shutdown");
        }
    }
}

#[cfg(test)]
mod test_worker {
    use super::*;

    #[test]
    fn test_can_parse_module_version() {
        let value = json!(
            {
                "moduleId": "test",
                "id": "ZVnigLgKIJQ1d_nmeT71g",
                "type": "HTTP",
                "handlers": {
                  "onRequest": "ZVnigLgKIJQ1d_nmeT71g"
                },
                "imports": {
                    "something": {
                        "type": "cloud",
                        "id": "test",
                        "versionId": "test"
                    }
                },
                "instructions": [
                  {
                    "type": "defineFunction",
                    "id": "ZVnigLgKIJQ1d_nmeT71g",
                    "body": [
                      {
                        "id": "jMwI6DurROzkjebyNhNyD",
                        "type": "call",
                        "callee": "import/http_server/respond_with",
                        "version": "1",
                        "parameters": [
                          {
                            "name": "body",
                            "type": "tel",
                            "expression": "{\n  message: \"Hello World\"\n}"
                          },
                          {
                            "name": "responseType",
                            "type": "tel",
                            "expression": "\"json\""
                          }
                        ]
                      }
                    ],
                    "name": "Handle request",
                    "version": "1.0.0",
                    "parameters": [
                      {
                        "name": "request",
                        "optional": false,
                        "description": "The incoming request object."
                      }
                    ]
                  }
                ]
              }
        );

        serde_json::from_value::<ModuleVersion>(value).unwrap();
    }

    #[test]
    fn test_can_parse_empty_module_version() {
        let value = json!(
            {
                "id": "ZVnigLgKIJQ1d_nmeT71g",
                "type": "HTTP",
                "handlers": {},
                "imports": {},
                "instructions": [],
                "moduleId": "test"
              }
        );

        serde_json::from_value::<ModuleVersion>(value).unwrap();
    }
}

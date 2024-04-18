use axum::{extract::ws::Message, response::Response};
use dashmap::DashMap;
use serde_derive::{Deserialize, Serialize};
use tel::StorageValue;

use std::{
    collections::HashMap,
    fmt::{self, Debug},
    sync::Arc,
};
use tokio::sync::{
    mpsc,
    oneshot::{self},
    Mutex,
};

use crate::{
    actions::redis::RedisPubSubCoordinatorHandle, actor::ActorCommand, errors::ExecutionError,
};

const WEBSOCKET_RESOURCE_TYPE: &str = "websocket";
const POSTGRES_CONNECTION_RESOURCE_TYPE: &str = "postgres_connection";
const REDIS_CONNECTION_RESOURCE_TYPE: &str = "redis_connection";
const HTTP_REQUEST_RESOURCE_TYPE: &str = "http_request";
const ACTOR_LINK_TYPE: &str = "actor_link";
const CANCELLATION: &str = "cancellation";
const FILE_HANDLE: &str = "file_handle";

pub trait Resource {
    fn get_type() -> &'static str;
}

// Machine resources
pub struct RedisPool(pub deadpool_redis::Pool, pub RedisPubSubCoordinatorHandle);

impl Resource for RedisPool {
    fn get_type() -> &'static str {
        REDIS_CONNECTION_RESOURCE_TYPE
    }
}

impl Debug for RedisPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedisPool")
            .field("0", &"RedisPool")
            .finish()
    }
}

#[derive(Debug)]
pub struct PostgresPool(pub deadpool_postgres::Pool);

impl Resource for PostgresPool {
    fn get_type() -> &'static str {
        POSTGRES_CONNECTION_RESOURCE_TYPE
    }
}

#[derive(Debug)]
pub struct OpenWebSocket(pub mpsc::Sender<WebSocketCommand>);

impl Resource for OpenWebSocket {
    fn get_type() -> &'static str {
        WEBSOCKET_RESOURCE_TYPE
    }
}

#[derive(Debug)]
pub struct ActorLink(pub mpsc::Sender<ActorCommand>);

impl Resource for ActorLink {
    fn get_type() -> &'static str {
        ACTOR_LINK_TYPE
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub method: String,
    pub path: String,
    #[serde(rename = "moduleVersionId")]
    pub module_version_id: String,
    pub handlers: HashMap<String, String>,
}

#[derive(Debug, Default)]
pub struct RegisteringRouter {
    routes: Vec<Route>,
}

impl RegisteringRouter {
    pub fn add_route(
        &mut self,
        method: String,
        path: String,
        module_version_id: String,
        handlers: HashMap<String, String>,
    ) {
        self.routes.push(Route {
            method,
            path,
            module_version_id,
            handlers,
        });
    }

    pub fn get_routes(&self) -> &Vec<Route> {
        &self.routes
    }

    pub fn clear(&mut self) {
        self.routes.clear();
    }
}

// Local resources

#[derive(Debug)]
pub enum HttpResponse {
    Normal(Response, oneshot::Sender<Result<(), ExecutionError>>),
    WebSocket(oneshot::Sender<Result<(), ExecutionError>>),
}

impl HttpResponse {
    pub fn new(response: Response) -> (Self, oneshot::Receiver<Result<(), ExecutionError>>) {
        let (responder, receiver) = oneshot::channel();
        let http_response = HttpResponse::Normal(response, responder);
        (http_response, receiver)
    }

    pub fn new_ws() -> (Self, oneshot::Receiver<Result<(), ExecutionError>>) {
        let (responder, receiver) = oneshot::channel();
        let http_response = HttpResponse::WebSocket(responder);
        (http_response, receiver)
    }
}

#[derive(Debug)]
pub struct WebSocketAcceptance {
    pub name: Option<String>,
    pub metadata: StorageValue,
    pub responder: oneshot::Sender<Result<(), ExecutionError>>,
}

impl WebSocketAcceptance {
    pub fn new(
        name: Option<String>,
        metadata: StorageValue,
    ) -> (Self, oneshot::Receiver<Result<(), ExecutionError>>) {
        let (responder, receiver) = oneshot::channel();
        let acceptance = Self {
            name,
            metadata,
            responder,
        };
        (acceptance, receiver)
    }
}

#[derive(Debug)]
pub struct WebSocketCommand {
    pub message: Message,
    pub responder: oneshot::Sender<Result<(), ExecutionError>>,
}

impl WebSocketCommand {
    pub fn new(message: Message) -> (Self, oneshot::Receiver<Result<(), ExecutionError>>) {
        let (responder, receiver) = oneshot::channel();
        let command = Self { message, responder };
        (command, receiver)
    }
}

#[derive(Debug)]
pub struct HttpRequestToRespond(pub oneshot::Sender<HttpResponse>);

impl Resource for HttpRequestToRespond {
    fn get_type() -> &'static str {
        HTTP_REQUEST_RESOURCE_TYPE
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CancellationSubject {
    Unknown,
    Alarm,
    Cronjob,
    PubSubSubscription,
    RedisPubSubSubscription,
    PostgresNotificationSubscription,
}

#[derive(Debug)]
pub struct Cancellation {
    pub sender: oneshot::Sender<()>,

    /// Name of the cancellation. Used for choosing which cancellations to cancel.
    pub name: String,

    /// Subject of the cancellation, basically we the system is going to cancel
    pub subject: CancellationSubject,
}

impl Resource for Cancellation {
    fn get_type() -> &'static str {
        CANCELLATION
    }
}

#[derive(Debug)]
pub struct FileHandle(pub tokio::fs::File);

impl Resource for FileHandle {
    fn get_type() -> &'static str {
        FILE_HANDLE
    }
}

#[derive(Debug, Default)]
pub struct ResourceRegistry {
    pub redis_pools: DashMap<String, RedisPool>,
    pub postgres_pools: DashMap<String, PostgresPool>,
    pub actors: DashMap<String, ActorLink>,
    pub router: Arc<Mutex<RegisteringRouter>>,
}

#[derive(Debug, Default)]
pub struct ActorResources {
    pub websockets: Vec<OpenWebSocket>,
    pub http_requests_to_respond: Vec<HttpRequestToRespond>,
    pub cancellations: Vec<Cancellation>,
    pub files: Vec<FileHandle>,
}

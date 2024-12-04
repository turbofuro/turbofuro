use axum::{
    body::{Body, BodyDataStream, Bytes},
    extract::ws::Message,
    response::{
        sse::{self},
        Response,
    },
};
use dashmap::DashMap;
use futures_util::stream::Stream;
use futures_util::{StreamExt, TryStreamExt};
use reqwest::multipart::Form;
use serde_derive::{Deserialize, Serialize};
use std::{convert::Infallible, pin::Pin};
use tel::StorageValue;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::ReaderStream;

use std::{
    collections::HashMap,
    fmt::{self, Debug},
    sync::Arc,
};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::{
    actions::redis::RedisPubSubCoordinatorHandle, actor::ActorCommand, errors::ExecutionError,
};

const WEBSOCKET_RESOURCE_TYPE: &str = "websocket";
const SSE_RESOURCE_TYPE: &str = "sse";
const POSTGRES_CONNECTION_RESOURCE_TYPE: &str = "postgres_connection";
const REDIS_CONNECTION_RESOURCE_TYPE: &str = "redis_connection";
const HTTP_CLIENT_RESOURCE_TYPE: &str = "http_client";
const HTTP_REQUEST_RESOURCE_TYPE: &str = "http_request";
const PENDING_HTTP_RESPONSE_TYPE: &str = "pending_http_response";
const PENDING_HTTP_REQUEST_TYPE: &str = "pending_http_request";
const FORM_DATA_DRAFT_TYPE: &str = "form_data_draft";
const PENDING_FORM_DATA_TYPE: &str = "pending_form_data";
const PENDING_FORM_DATA_FIELD_TYPE: &str = "pending_form_data_field";
const ACTOR_LINK_TYPE: &str = "actor_link";
const CANCELLATION_TYPE: &str = "cancellation";
const FILE_HANDLE_TYPE: &str = "file_handle";
const LIBSQL_CONNECTION_TYPE: &str = "libsql_connection";
const WEBDRIVER_CLIENT_TYPE: &str = "webdriver_client";
const WEBDRIVER_ELEMENT_TYPE: &str = "webdriver_element";

pub trait Resource {
    fn get_type() -> &'static str;

    fn missing() -> ExecutionError {
        ExecutionError::MissingResource {
            resource_type: Self::get_type().into(),
        }
    }
}

//
// Machine resources
//

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

pub struct LibSql(pub libsql::Connection);

impl Resource for LibSql {
    fn get_type() -> &'static str {
        LIBSQL_CONNECTION_TYPE
    }
}

impl Debug for LibSql {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LibSql").field("0", &"LibSql").finish()
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
pub struct OpenSseStream(pub mpsc::Sender<Result<sse::Event, Infallible>>);

impl Resource for OpenSseStream {
    fn get_type() -> &'static str {
        SSE_RESOURCE_TYPE
    }
}

#[derive(Debug, Clone)]
pub struct ActorLink {
    pub sender: mpsc::Sender<ActorCommand>,
    pub module_id: String,
}

impl ActorLink {
    pub fn new(sender: mpsc::Sender<ActorCommand>, module_id: String) -> Self {
        Self { sender, module_id }
    }

    pub async fn send(&self, command: ActorCommand) -> Result<(), ExecutionError> {
        self.sender
            .send(command)
            .await
            .map_err(ExecutionError::from)
    }
}

impl Resource for ActorLink {
    fn get_type() -> &'static str {
        ACTOR_LINK_TYPE
    }
}

#[derive(Clone, Debug)]
pub struct HttpClient(pub reqwest::Client);

impl Resource for HttpClient {
    fn get_type() -> &'static str {
        HTTP_CLIENT_RESOURCE_TYPE
    }
}

#[derive(Clone, Debug)]
pub struct WebDriverClient(pub fantoccini::Client);

impl Resource for WebDriverClient {
    fn get_type() -> &'static str {
        &WEBDRIVER_CLIENT_TYPE
    }
}

#[derive(Clone, Debug)]
pub struct WebDriverElement(pub fantoccini::elements::Element);

impl Resource for WebDriverElement {
    fn get_type() -> &'static str {
        &WEBDRIVER_ELEMENT_TYPE
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub method: String,
    pub path: String,
    #[serde(rename = "moduleVersionId")]
    pub module_version_id: String,
    pub handlers: HashMap<String, String>,
    pub parse_body: bool,
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
            parse_body: true,
        });
    }

    pub fn add_streaming_route(
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
            parse_body: false,
        });
    }

    pub fn get_routes(&self) -> &Vec<Route> {
        &self.routes
    }

    pub fn clear(&mut self) {
        self.routes.clear();
    }
}

//
// Local resources
//

#[derive(Debug)]
pub enum HttpResponse {
    Normal(Response, oneshot::Sender<Result<(), ExecutionError>>),
    WebSocket(oneshot::Sender<Result<(), ExecutionError>>),
    ServerSentEvents {
        event_receiver: mpsc::Receiver<Result<sse::Event, Infallible>>,
        keep_alive: Option<sse::KeepAlive>,
        disconnect_sender: oneshot::Sender<StorageValue>,
        responder: oneshot::Sender<Result<(), ExecutionError>>,
    },
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

    pub fn new_sse(
        event_receiver: mpsc::Receiver<Result<sse::Event, Infallible>>,
        keep_alive: Option<sse::KeepAlive>,
        disconnect_sender: oneshot::Sender<StorageValue>,
    ) -> (Self, oneshot::Receiver<Result<(), ExecutionError>>) {
        let (responder, receiver) = oneshot::channel();
        let http_response = HttpResponse::ServerSentEvents {
            event_receiver,
            keep_alive,
            disconnect_sender,
            responder,
        };
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
pub struct HttpRequestToRespond {
    pub response_sender: oneshot::Sender<HttpResponse>,
}

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
    Watcher,
    Task,
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
        CANCELLATION_TYPE
    }
}

#[derive(Debug)]
pub struct FileHandle {
    pub file: tokio::fs::File,
}

impl Resource for FileHandle {
    fn get_type() -> &'static str {
        FILE_HANDLE_TYPE
    }
}

#[derive(Debug)]
pub struct PendingHttpResponseBody(pub reqwest::Response);

impl PendingHttpResponseBody {
    pub fn new(response: reqwest::Response) -> Self {
        Self(response)
    }
}

impl Resource for PendingHttpResponseBody {
    fn get_type() -> &'static str {
        PENDING_HTTP_RESPONSE_TYPE
    }
}

#[derive(Debug)]
pub struct PendingHttpRequestBody(pub Body);

impl PendingHttpRequestBody {
    pub fn new(body: Body) -> Self {
        Self(body)
    }
}

impl Resource for PendingHttpRequestBody {
    fn get_type() -> &'static str {
        PENDING_HTTP_REQUEST_TYPE
    }
}

#[derive(Debug)]
pub struct FormDataDraft(pub Form);

impl FormDataDraft {
    pub fn new(form: Form) -> Self {
        Self(form)
    }
}

impl Resource for FormDataDraft {
    fn get_type() -> &'static str {
        FORM_DATA_DRAFT_TYPE
    }
}

#[derive(Debug)]
pub enum MultipartManagerFieldEvent {
    Error,
    Empty,
    File {
        name: Option<String>,
        filename: Option<String>,
        index: usize,
        headers: HashMap<String, String>,
        receiver: tokio::sync::mpsc::Receiver<Result<Bytes, ExecutionError>>,
    },
}

#[derive(Debug)]
pub enum MultipartManagerCommand {
    GetNext {
        sender: oneshot::Sender<MultipartManagerFieldEvent>,
    },
}

#[derive(Debug)]
pub struct PendingFormData(pub mpsc::Sender<MultipartManagerCommand>);

impl Resource for PendingFormData {
    fn get_type() -> &'static str {
        PENDING_FORM_DATA_TYPE
    }
}

#[derive(Debug)]
pub struct PendingFormDataField(pub mpsc::Receiver<Result<Bytes, ExecutionError>>);

impl Resource for PendingFormDataField {
    fn get_type() -> &'static str {
        PENDING_FORM_DATA_FIELD_TYPE
    }
}

#[derive(Debug, Default)]
pub struct ResourceRegistry {
    pub redis_pools: DashMap<String, RedisPool>,
    pub postgres_pools: DashMap<String, PostgresPool>,
    pub actors: DashMap<String, ActorLink>,
    pub http_clients: DashMap<String, HttpClient>,
    pub router: Arc<Mutex<RegisteringRouter>>,
    pub libsql: DashMap<String, LibSql>,
}

#[derive(Debug, Default)]
pub struct ActorResources {
    pub websockets: Vec<OpenWebSocket>,
    pub sse_streams: Vec<OpenSseStream>,
    pub http_requests_to_respond: Vec<HttpRequestToRespond>,
    pub cancellations: Vec<Cancellation>,
    pub files: Vec<FileHandle>,
    pub pending_response_body: Vec<PendingHttpResponseBody>,
    pub pending_request_body: Vec<PendingHttpRequestBody>,
    pub form_data: Vec<FormDataDraft>,
    pub pending_form_data: Vec<PendingFormData>,
    pub pending_form_data_fields: Vec<PendingFormDataField>,
    pub webdriver_clients: Vec<WebDriverClient>,
    pub webdriver_elements: Vec<WebDriverElement>,
}

impl ActorResources {
    pub fn append(&mut self, other: &mut ActorResources) {
        self.http_requests_to_respond
            .append(&mut other.http_requests_to_respond);
        self.websockets.append(&mut other.websockets);
        self.cancellations.append(&mut other.cancellations);
        self.files.append(&mut other.files);
        self.pending_response_body
            .append(&mut other.pending_response_body);
        self.pending_request_body
            .append(&mut other.pending_request_body);
        self.form_data.append(&mut other.form_data);
        self.pending_form_data.append(&mut other.pending_form_data);
        self.pending_form_data_fields
            .append(&mut other.pending_form_data_fields);
        self.webdriver_clients.append(&mut other.webdriver_clients);
    }
}

pub struct PendingHttpRequestStream(pub BodyDataStream);

// TODO: Figure out how to fix this non-sense
// https://github.com/tokio-rs/axum/discussions/2540
unsafe impl Sync for PendingHttpRequestStream {}

impl Stream for PendingHttpRequestStream {
    type Item = Result<Bytes, ExecutionError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.0
            .poll_next_unpin(cx)
            .map_err(|e| ExecutionError::IoError {
                message: e.to_string(),
                os_code: None,
            })
    }
}

type HammerStream = Pin<Box<dyn Stream<Item = Result<Bytes, ExecutionError>> + Send + Sync>>;

impl ActorResources {
    pub fn get_nearest_stream(&mut self) -> Result<HammerStream, ExecutionError> {
        // TODO: Implement a stack-like behavior for choosing the nearest stream
        // Currently, it just pops the first response, file... etc.

        let request = self.pending_request_body.pop();
        if let Some(request) = request {
            let stream = PendingHttpRequestStream(request.0.into_data_stream());
            return Ok(Box::pin(stream));
        }

        let pending_form_data_fields = self.pending_form_data_fields.pop();
        if let Some(pending_form_data_fields) = pending_form_data_fields {
            return Ok(Box::pin(ReceiverStream::new(pending_form_data_fields.0)));
        }

        let response = self.pending_response_body.pop();
        if let Some(response) = response {
            let a = response
                .0
                .bytes_stream()
                .map_err(|e| ExecutionError::IoError {
                    message: e.to_string(),
                    os_code: None,
                });

            return Ok(Box::pin(a));
        }

        let file_handle = self.files.pop();
        if let Some(file_handle) = file_handle {
            return Ok(Box::pin(ReaderStream::new(file_handle.file).map_err(|e| {
                ExecutionError::IoError {
                    message: e.to_string(),
                    os_code: None,
                }
            })));
        }

        Err(ExecutionError::MissingResource {
            resource_type: FILE_HANDLE_TYPE.to_string(),
        })
    }
}

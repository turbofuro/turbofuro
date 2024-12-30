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
use std::{convert::Infallible, pin::Pin, sync::atomic::AtomicU64};
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

    fn get_id(&self) -> ResourceId;
}

// Atomic counter for generating unique IDs
static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

pub type ResourceId = u64;

pub fn generate_resource_id() -> u64 {
    ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}

pub struct RedisPool(
    pub ResourceId,
    pub deadpool_redis::Pool,
    pub RedisPubSubCoordinatorHandle,
);

impl Resource for RedisPool {
    fn get_type() -> &'static str {
        REDIS_CONNECTION_RESOURCE_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.0
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
pub struct PostgresPool(pub ResourceId, pub deadpool_postgres::Pool);

impl Resource for PostgresPool {
    fn get_type() -> &'static str {
        POSTGRES_CONNECTION_RESOURCE_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.0
    }
}

pub struct LibSql(pub ResourceId, pub libsql::Connection);

impl Resource for LibSql {
    fn get_type() -> &'static str {
        LIBSQL_CONNECTION_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.0
    }
}

impl Debug for LibSql {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LibSql").field("0", &"LibSql").finish()
    }
}

#[derive(Debug)]
pub struct OpenWebSocket(pub ResourceId, pub mpsc::Sender<WebSocketCommand>);

impl Resource for OpenWebSocket {
    fn get_type() -> &'static str {
        WEBSOCKET_RESOURCE_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.0
    }
}

#[derive(Debug)]
pub struct OpenSseStream(
    pub ResourceId,
    pub mpsc::Sender<Result<sse::Event, Infallible>>,
);

impl Resource for OpenSseStream {
    fn get_type() -> &'static str {
        SSE_RESOURCE_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.0
    }
}

#[derive(Debug, Clone)]
pub struct ActorLink {
    pub id: ResourceId,
    pub sender: mpsc::Sender<ActorCommand>,
    pub module_id: String,
}

impl ActorLink {
    pub fn new(sender: mpsc::Sender<ActorCommand>, module_id: String) -> Self {
        Self {
            sender,
            module_id,
            id: generate_resource_id(),
        }
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

    fn get_id(&self) -> ResourceId {
        self.id
    }
}

#[derive(Clone, Debug)]
pub struct HttpClient(pub ResourceId, pub reqwest::Client);

impl Resource for HttpClient {
    fn get_type() -> &'static str {
        HTTP_CLIENT_RESOURCE_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.0
    }
}

#[derive(Clone, Debug)]
pub struct WebDriverClient(pub ResourceId, pub fantoccini::Client);

impl Resource for WebDriverClient {
    fn get_type() -> &'static str {
        &WEBDRIVER_CLIENT_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.0
    }
}

#[derive(Clone, Debug)]
pub struct WebDriverElement(pub ResourceId, pub fantoccini::elements::Element);

impl Resource for WebDriverElement {
    fn get_type() -> &'static str {
        &WEBDRIVER_ELEMENT_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.0
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
    pub id: ResourceId,
    pub response_sender: oneshot::Sender<HttpResponse>,
}

impl Resource for HttpRequestToRespond {
    fn get_type() -> &'static str {
        HTTP_REQUEST_RESOURCE_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.id
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
    pub id: ResourceId,
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

    fn get_id(&self) -> ResourceId {
        self.id
    }
}

#[derive(Debug)]
pub struct FileHandle {
    pub id: ResourceId,
    pub file: tokio::fs::File,
}

impl Resource for FileHandle {
    fn get_type() -> &'static str {
        FILE_HANDLE_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.id
    }
}

#[derive(Debug)]
pub struct PendingHttpResponseBody(pub ResourceId, pub reqwest::Response);

impl PendingHttpResponseBody {
    pub fn new(response: reqwest::Response) -> Self {
        Self(generate_resource_id(), response)
    }
}

impl Resource for PendingHttpResponseBody {
    fn get_type() -> &'static str {
        PENDING_HTTP_RESPONSE_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.0
    }
}

#[derive(Debug)]
pub struct PendingHttpRequestBody(pub ResourceId, pub Body);

impl PendingHttpRequestBody {
    pub fn new(body: Body) -> Self {
        Self(generate_resource_id(), body)
    }
}

impl Resource for PendingHttpRequestBody {
    fn get_type() -> &'static str {
        PENDING_HTTP_REQUEST_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.0
    }
}

#[derive(Debug)]
pub struct FormDataDraft(pub ResourceId, pub Form);

impl FormDataDraft {
    pub fn new(form: Form) -> Self {
        Self(generate_resource_id(), form)
    }
}

impl Resource for FormDataDraft {
    fn get_type() -> &'static str {
        FORM_DATA_DRAFT_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.0
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
pub struct PendingFormData(pub ResourceId, pub mpsc::Sender<MultipartManagerCommand>);

impl PendingFormData {
    pub fn new(sender: mpsc::Sender<MultipartManagerCommand>) -> Self {
        Self(generate_resource_id(), sender)
    }
}

impl Resource for PendingFormData {
    fn get_type() -> &'static str {
        PENDING_FORM_DATA_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.0
    }
}

#[derive(Debug)]
pub struct PendingFormDataField(
    pub ResourceId,
    pub mpsc::Receiver<Result<Bytes, ExecutionError>>,
);

impl Resource for PendingFormDataField {
    fn get_type() -> &'static str {
        PENDING_FORM_DATA_FIELD_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.0
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

#[derive(Debug)]
pub struct StreamableResource {
    pub resource: &'static str,
    pub id: ResourceId,
}

#[derive(Debug, Default)]
pub struct ActorResources {
    websockets: Vec<OpenWebSocket>,
    sse_streams: Vec<OpenSseStream>,
    http_requests_to_respond: Vec<HttpRequestToRespond>,
    cancellations: Vec<Cancellation>,
    files: Vec<FileHandle>,
    pending_response_body: Vec<PendingHttpResponseBody>,
    pending_request_body: Vec<PendingHttpRequestBody>,
    form_data: Vec<FormDataDraft>,
    pending_form_data: Vec<PendingFormData>,
    pending_form_data_fields: Vec<PendingFormDataField>,
    webdriver_clients: Vec<WebDriverClient>,
    webdriver_elements: Vec<WebDriverElement>,
    streams: Vec<StreamableResource>,
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
        self.webdriver_elements
            .append(&mut other.webdriver_elements);
        self.streams.append(&mut other.streams);
        self.sse_streams.append(&mut other.sse_streams);
        self.http_requests_to_respond
            .append(&mut other.http_requests_to_respond);
    }

    pub fn add_websocket(&mut self, websocket: OpenWebSocket) {
        self.websockets.push(websocket);
    }

    pub fn use_websocket<F>(&mut self, f: F) -> Option<&mut OpenWebSocket>
    where
        F: Fn(&OpenWebSocket) -> bool,
    {
        self.websockets
            .iter()
            .rposition(f)
            .map(|i| &mut self.websockets[i])
    }

    pub fn pop_websocket(&mut self) -> Option<OpenWebSocket> {
        self.websockets.pop()
    }

    pub fn pop_websocket_where<F>(&mut self, f: F) -> Option<OpenWebSocket>
    where
        F: Fn(&OpenWebSocket) -> bool,
    {
        self.websockets
            .iter()
            .rposition(f)
            .map(|i| self.websockets.remove(i))
    }

    pub fn add_sse_stream(&mut self, sse_stream: OpenSseStream) {
        self.sse_streams.push(sse_stream);
    }

    pub fn use_sse_stream<F>(&mut self, f: F) -> Option<&mut OpenSseStream>
    where
        F: Fn(&OpenSseStream) -> bool,
    {
        self.sse_streams
            .iter()
            .rposition(f)
            .map(|i| &mut self.sse_streams[i])
    }

    pub fn pop_sse_stream(&mut self) -> Option<OpenSseStream> {
        self.sse_streams.pop()
    }

    pub fn pop_sse_stream_where<F>(&mut self, f: F) -> Option<OpenSseStream>
    where
        F: Fn(&OpenSseStream) -> bool,
    {
        self.sse_streams
            .iter()
            .rposition(f)
            .map(|i| self.sse_streams.remove(i))
    }

    pub fn add_http_request_to_respond(&mut self, http_request_to_respond: HttpRequestToRespond) {
        self.http_requests_to_respond.push(http_request_to_respond);
    }

    pub fn use_http_request_to_respond<F>(&mut self, f: F) -> Option<&mut HttpRequestToRespond>
    where
        F: Fn(&HttpRequestToRespond) -> bool,
    {
        self.http_requests_to_respond
            .iter()
            .rposition(f)
            .map(|i| &mut self.http_requests_to_respond[i])
    }

    pub fn pop_http_request_to_respond(&mut self) -> Option<HttpRequestToRespond> {
        self.http_requests_to_respond.pop()
    }

    pub fn pop_http_request_to_respond_where<F>(&mut self, f: F) -> Option<HttpRequestToRespond>
    where
        F: Fn(&HttpRequestToRespond) -> bool,
    {
        self.http_requests_to_respond
            .iter()
            .rposition(f)
            .map(|i| self.http_requests_to_respond.remove(i))
    }

    pub fn add_cancellation(&mut self, cancellation: Cancellation) {
        self.cancellations.push(cancellation);
    }

    pub fn use_cancellation<F>(&mut self, f: F) -> Option<&mut Cancellation>
    where
        F: Fn(&Cancellation) -> bool,
    {
        self.cancellations
            .iter()
            .rposition(f)
            .map(|i| &mut self.cancellations[i])
    }

    pub fn pop_cancellation(&mut self) -> Option<Cancellation> {
        self.cancellations.pop()
    }

    pub fn pop_cancellation_where<F>(&mut self, f: F) -> Option<Cancellation>
    where
        F: Fn(&Cancellation) -> bool,
    {
        self.cancellations
            .iter()
            .rposition(f)
            .map(|i| self.cancellations.remove(i))
    }

    pub fn add_file(&mut self, file: FileHandle) {
        self.add_streamable(FileHandle::get_type(), file.get_id());
        self.files.push(file);
    }

    pub fn use_file<F>(&mut self, f: F) -> Option<&mut FileHandle>
    where
        F: Fn(&FileHandle) -> bool,
    {
        self.files.iter().rposition(f).map(|i| &mut self.files[i])
    }

    pub fn pop_file(&mut self) -> Option<FileHandle> {
        self.files.pop().inspect(|r| {
            self.clear_streamable(r.get_id());
        })
    }

    pub fn pop_file_where<F>(&mut self, f: F) -> Option<FileHandle>
    where
        F: Fn(&FileHandle) -> bool,
    {
        self.files
            .iter()
            .rposition(f)
            .map(|i| self.files.remove(i))
            .inspect(|r| {
                self.clear_streamable(r.get_id());
            })
    }

    pub fn add_pending_response_body(&mut self, pending_response_body: PendingHttpResponseBody) {
        self.add_streamable(
            PendingHttpResponseBody::get_type(),
            pending_response_body.get_id(),
        );
        self.pending_response_body.push(pending_response_body);
    }

    pub fn use_pending_response_body<F>(&mut self, f: F) -> Option<&mut PendingHttpResponseBody>
    where
        F: Fn(&PendingHttpResponseBody) -> bool,
    {
        self.pending_response_body
            .iter()
            .rposition(f)
            .map(|i| &mut self.pending_response_body[i])
    }

    pub fn pop_pending_response_body(&mut self) -> Option<PendingHttpResponseBody> {
        self.pending_response_body.pop().inspect(|r| {
            self.clear_streamable(r.get_id());
        })
    }

    pub fn pop_pending_response_body_where<F>(&mut self, f: F) -> Option<PendingHttpResponseBody>
    where
        F: Fn(&PendingHttpResponseBody) -> bool,
    {
        self.pending_response_body
            .iter()
            .rposition(f)
            .map(|i| self.pending_response_body.remove(i))
            .inspect(|r| {
                self.clear_streamable(r.get_id());
            })
    }

    pub fn add_pending_request_body(&mut self, pending_request_body: PendingHttpRequestBody) {
        self.add_streamable(
            PendingHttpRequestBody::get_type(),
            pending_request_body.get_id(),
        );
        self.pending_request_body.push(pending_request_body);
    }

    pub fn use_pending_request_body<F>(&mut self, f: F) -> Option<&mut PendingHttpRequestBody>
    where
        F: Fn(&PendingHttpRequestBody) -> bool,
    {
        self.pending_request_body
            .iter()
            .rposition(f)
            .map(|i| &mut self.pending_request_body[i])
    }

    pub fn pop_pending_request_body(&mut self) -> Option<PendingHttpRequestBody> {
        self.pending_request_body.pop().inspect(|r| {
            self.clear_streamable(r.get_id());
        })
    }

    pub fn pop_pending_request_body_where<F>(&mut self, f: F) -> Option<PendingHttpRequestBody>
    where
        F: Fn(&PendingHttpRequestBody) -> bool,
    {
        self.pending_request_body
            .iter()
            .rposition(f)
            .map(|i| self.pending_request_body.remove(i))
            .inspect(|r| {
                self.clear_streamable(r.get_id());
            })
    }

    pub fn add_form_data(&mut self, form_data: FormDataDraft) {
        self.form_data.push(form_data);
    }

    pub fn use_form_data<F>(&mut self, f: F) -> Option<&mut FormDataDraft>
    where
        F: Fn(&FormDataDraft) -> bool,
    {
        self.form_data
            .iter()
            .rposition(f)
            .map(|i| &mut self.form_data[i])
    }

    pub fn pop_form_data(&mut self) -> Option<FormDataDraft> {
        self.form_data.pop()
    }

    pub fn pop_form_data_where<F>(&mut self, f: F) -> Option<FormDataDraft>
    where
        F: Fn(&FormDataDraft) -> bool,
    {
        self.form_data
            .iter()
            .rposition(f)
            .map(|i| self.form_data.remove(i))
    }

    pub fn add_pending_form_data(&mut self, pending_form_data: PendingFormData) {
        self.pending_form_data.push(pending_form_data);
    }

    pub fn use_pending_form_data<F>(&mut self, f: F) -> Option<&mut PendingFormData>
    where
        F: Fn(&PendingFormData) -> bool,
    {
        self.pending_form_data
            .iter()
            .rposition(f)
            .map(|i| &mut self.pending_form_data[i])
    }

    pub fn pop_pending_form_data(&mut self) -> Option<PendingFormData> {
        self.pending_form_data.pop()
    }

    pub fn pop_pending_form_data_where<F>(&mut self, f: F) -> Option<PendingFormData>
    where
        F: Fn(&PendingFormData) -> bool,
    {
        self.pending_form_data
            .iter()
            .rposition(f)
            .map(|i| self.pending_form_data.remove(i))
    }

    pub fn add_pending_form_data_field(&mut self, pending_form_data_field: PendingFormDataField) {
        self.add_streamable(
            PendingFormDataField::get_type(),
            pending_form_data_field.get_id(),
        );
        self.pending_form_data_fields.push(pending_form_data_field);
    }

    pub fn use_pending_form_data_field<F>(&mut self, f: F) -> Option<&mut PendingFormDataField>
    where
        F: Fn(&PendingFormDataField) -> bool,
    {
        self.pending_form_data_fields
            .iter()
            .rposition(f)
            .map(|i| &mut self.pending_form_data_fields[i])
    }

    pub fn pop_pending_form_data_field(&mut self) -> Option<PendingFormDataField> {
        self.pending_form_data_fields.pop().inspect(|r| {
            self.clear_streamable(r.get_id());
        })
    }

    pub fn pop_pending_form_data_field_where<F>(&mut self, f: F) -> Option<PendingFormDataField>
    where
        F: Fn(&PendingFormDataField) -> bool,
    {
        self.pending_form_data_fields
            .iter()
            .rposition(f)
            .map(|i| self.pending_form_data_fields.remove(i))
            .inspect(|r| {
                self.clear_streamable(r.get_id());
            })
    }

    pub fn add_webdriver_client(&mut self, webdriver_client: WebDriverClient) {
        self.webdriver_clients.push(webdriver_client);
    }

    pub fn use_webdriver_client<F>(&mut self, f: F) -> Option<&mut WebDriverClient>
    where
        F: Fn(&WebDriverClient) -> bool,
    {
        self.webdriver_clients
            .iter()
            .rposition(f)
            .map(|i| &mut self.webdriver_clients[i])
    }

    pub fn pop_webdriver_client(&mut self) -> Option<WebDriverClient> {
        self.webdriver_clients.pop()
    }

    pub fn pop_webdriver_client_where<F>(&mut self, f: F) -> Option<WebDriverClient>
    where
        F: Fn(&WebDriverClient) -> bool,
    {
        self.webdriver_clients
            .iter()
            .rposition(f)
            .map(|i| self.webdriver_clients.remove(i))
    }

    pub fn add_webdriver_element(&mut self, webdriver_element: WebDriverElement) {
        self.webdriver_elements.push(webdriver_element);
    }

    pub fn use_webdriver_element<F>(&mut self, f: F) -> Option<&mut WebDriverElement>
    where
        F: Fn(&WebDriverElement) -> bool,
    {
        self.webdriver_elements
            .iter()
            .rposition(f)
            .map(|i| &mut self.webdriver_elements[i])
    }

    pub fn pop_webdriver_element(&mut self) -> Option<WebDriverElement> {
        self.webdriver_elements.pop()
    }

    pub fn pop_webdriver_element_where<F>(&mut self, f: F) -> Option<WebDriverElement>
    where
        F: Fn(&WebDriverElement) -> bool,
    {
        self.webdriver_elements
            .iter()
            .rposition(f)
            .map(|i| self.webdriver_elements.remove(i))
    }

    fn clear_streamable(&mut self, id: ResourceId) -> Option<StreamableResource> {
        self.streams
            .iter()
            .rposition(|r| r.id == id)
            .map(|i| self.streams.remove(i))
    }

    fn add_streamable(&mut self, resource: &'static str, id: ResourceId) {
        self.streams.push(StreamableResource { resource, id })
    }

    pub fn get_stream(&mut self) -> Result<HammerStream, ExecutionError> {
        let last = self
            .streams
            .pop()
            .ok_or_else(|| ExecutionError::MissingResource {
                resource_type: "streamable".to_owned(),
            })?;

        match last.resource {
            PENDING_HTTP_REQUEST_TYPE => {
                let resource = self
                    .pop_pending_request_body_where(|r| r.0 == last.id)
                    .ok_or_else(|| ExecutionError::MissingResource {
                        resource_type: "streamable".to_owned(),
                    })?;

                let stream = PendingHttpRequestStream(resource.1.into_data_stream());
                Ok(Box::pin(stream))
            }
            PENDING_HTTP_RESPONSE_TYPE => {
                let resource = self
                    .pop_pending_response_body_where(|r| r.0 == last.id)
                    .ok_or_else(|| ExecutionError::MissingResource {
                        resource_type: "streamable".to_owned(),
                    })?;

                Ok(Box::pin(resource.1.bytes_stream().map_err(|e| {
                    ExecutionError::IoError {
                        message: e.to_string(),
                        os_code: None,
                    }
                })))
            }
            PENDING_FORM_DATA_FIELD_TYPE => {
                let resource = self
                    .pop_pending_form_data_field_where(|r| r.0 == last.id)
                    .ok_or_else(|| ExecutionError::MissingResource {
                        resource_type: "streamable".to_owned(),
                    })?;

                Ok(Box::pin(ReceiverStream::new(resource.1)))
            }
            FILE_HANDLE_TYPE => {
                let resource = self.pop_file_where(|r| r.id == last.id).ok_or_else(|| {
                    ExecutionError::MissingResource {
                        resource_type: "streamable".to_owned(),
                    }
                })?;

                Ok(Box::pin(ReaderStream::new(resource.file).map_err(|e| {
                    ExecutionError::IoError {
                        message: e.to_string(),
                        os_code: None,
                    }
                })))
            }
            _ => {
                unreachable!()
            }
        }
    }
}

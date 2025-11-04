use axum::{
    body::{BodyDataStream, Bytes},
    response::{
        sse::{self},
        Response,
    },
};
use dashmap::DashMap;
use futures_util::stream::Stream;
use futures_util::{StreamExt, TryStreamExt};
use serde_derive::{Deserialize, Serialize};
use std::{convert::Infallible, pin::Pin, sync::atomic::AtomicU64};
use tel::StorageValue;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::ReaderStream;

use std::{collections::HashMap, fmt::Debug, sync::Arc};
use tokio::sync::{mpsc, oneshot, Mutex};

pub use crate::{
    modules::{
        actors::ActorLink,
        fantoccini::{WebDriverClient, WebDriverElement},
        fs::{FileHandle, FILE_HANDLE_TYPE},
        http_client::{
            form_data::{FormDataDraft, FORM_DATA_DRAFT_TYPE},
            HttpClient, PendingHttpResponseBody, HTTP_CLIENT_RESOURCE_TYPE,
            PENDING_HTTP_RESPONSE_TYPE,
        },
        http_server::{
            form_data::{PendingFormData, PendingFormDataField, PENDING_FORM_DATA_FIELD_TYPE},
            HttpRequestToRespond, OpenSseStream, PendingHttpRequestBody,
            HTTP_REQUEST_RESOURCE_TYPE, PENDING_HTTP_REQUEST_TYPE, SSE_RESOURCE_TYPE,
        },
        libsql::LibSql,
        postgres::PostgresPool,
        redis::RedisPool,
        websocket_server::{OpenWebSocket, WebSocketCommand},
    },
    errors::ExecutionError,
};

const CANCELLATION_TYPE: &str = "cancellation";

pub trait Resource {
    fn static_type() -> &'static str;

    fn get_type(&self) -> &'static str {
        Self::static_type()
    }

    fn missing() -> ExecutionError {
        ExecutionError::MissingResource {
            resource_type: Self::static_type().into(),
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
    fn static_type() -> &'static str {
        CANCELLATION_TYPE
    }

    fn get_id(&self) -> ResourceId {
        self.id
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

pub type HammerStream = Pin<Box<dyn Stream<Item = Result<Bytes, ExecutionError>> + Send + Sync>>;

#[derive(Debug)]
pub struct SteamMetadata {
    pub id: ResourceId,
    pub type_: &'static str,
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

    pub fn use_websocket(&mut self) -> Option<&mut OpenWebSocket> {
        self.websockets.last_mut()
    }

    pub fn use_websocket_where<F>(&mut self, f: F) -> Option<&mut OpenWebSocket>
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

    pub fn use_sse_stream(&mut self) -> Option<&mut OpenSseStream> {
        self.sse_streams.last_mut()
    }

    pub fn use_sse_stream_where<F>(&mut self, f: F) -> Option<&mut OpenSseStream>
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

    pub fn use_http_request_to_respond(&mut self) -> Option<&mut HttpRequestToRespond> {
        self.http_requests_to_respond.last_mut()
    }

    pub fn use_http_request_to_respond_where<F>(
        &mut self,
        f: F,
    ) -> Option<&mut HttpRequestToRespond>
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

    pub fn use_cancellation(&mut self) -> Option<&mut Cancellation> {
        self.cancellations.last_mut()
    }

    pub fn use_cancellation_where<F>(&mut self, f: F) -> Option<&mut Cancellation>
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
        self.add_streamable(FileHandle::static_type(), file.get_id());
        self.files.push(file);
    }

    pub fn use_file(&mut self) -> Option<&mut FileHandle> {
        self.files.last_mut()
    }

    pub fn use_file_where<F>(&mut self, f: F) -> Option<&mut FileHandle>
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
            PendingHttpResponseBody::static_type(),
            pending_response_body.get_id(),
        );
        self.pending_response_body.push(pending_response_body);
    }

    pub fn use_pending_response_body(&mut self) -> Option<&mut PendingHttpResponseBody> {
        self.pending_response_body.last_mut()
    }

    pub fn use_pending_response_body_where<F>(
        &mut self,
        f: F,
    ) -> Option<&mut PendingHttpResponseBody>
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
            PendingHttpRequestBody::static_type(),
            pending_request_body.get_id(),
        );
        self.pending_request_body.push(pending_request_body);
    }

    pub fn use_pending_request_body(&mut self) -> Option<&mut PendingHttpRequestBody> {
        self.pending_request_body.last_mut()
    }

    pub fn use_pending_request_body_where<F>(&mut self, f: F) -> Option<&mut PendingHttpRequestBody>
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

    pub fn use_form_data(&mut self) -> Option<&mut FormDataDraft> {
        self.form_data.last_mut()
    }

    pub fn use_form_data_where<F>(&mut self, f: F) -> Option<&mut FormDataDraft>
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

    pub fn use_pending_form_data(&mut self) -> Option<&mut PendingFormData> {
        self.pending_form_data.last_mut()
    }

    pub fn use_pending_form_data_where<F>(&mut self, f: F) -> Option<&mut PendingFormData>
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
            PendingFormDataField::static_type(),
            pending_form_data_field.get_id(),
        );
        self.pending_form_data_fields.push(pending_form_data_field);
    }

    pub fn use_pending_form_data_field(&mut self) -> Option<&mut PendingFormDataField> {
        self.pending_form_data_fields.last_mut()
    }

    pub fn use_pending_form_data_field_where<F>(
        &mut self,
        f: F,
    ) -> Option<&mut PendingFormDataField>
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

    pub fn use_webdriver_client(&mut self) -> Option<&mut WebDriverClient> {
        self.webdriver_clients.last_mut()
    }

    pub fn use_webdriver_client_where<F>(&mut self, f: F) -> Option<&mut WebDriverClient>
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

    pub fn use_webdriver_element(&mut self) -> Option<&mut WebDriverElement> {
        self.webdriver_elements.last_mut()
    }

    pub fn use_webdriver_element_where<F>(&mut self, f: F) -> Option<&mut WebDriverElement>
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

    pub fn get_stream(&mut self) -> Result<(HammerStream, SteamMetadata), ExecutionError> {
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

                let metadata = SteamMetadata {
                    id: last.id,
                    type_: PENDING_HTTP_REQUEST_TYPE,
                };

                let stream = PendingHttpRequestStream(resource.1.into_data_stream());
                Ok((Box::pin(stream), metadata))
            }
            PENDING_HTTP_RESPONSE_TYPE => {
                let resource = self
                    .pop_pending_response_body_where(|r| r.0 == last.id)
                    .ok_or_else(|| ExecutionError::MissingResource {
                        resource_type: "streamable".to_owned(),
                    })?;

                let metadata = SteamMetadata {
                    id: last.id,
                    type_: PENDING_HTTP_RESPONSE_TYPE,
                };

                Ok((
                    Box::pin(
                        resource
                            .1
                            .bytes_stream()
                            .map_err(|e| ExecutionError::IoError {
                                message: e.to_string(),
                                os_code: None,
                            }),
                    ),
                    metadata,
                ))
            }
            PENDING_FORM_DATA_FIELD_TYPE => {
                let resource = self
                    .pop_pending_form_data_field_where(|r| r.0 == last.id)
                    .ok_or_else(|| ExecutionError::MissingResource {
                        resource_type: "streamable".to_owned(),
                    })?;

                let metadata = SteamMetadata {
                    id: last.id,
                    type_: PENDING_FORM_DATA_FIELD_TYPE,
                };

                Ok((Box::pin(ReceiverStream::new(resource.1)), metadata))
            }
            FILE_HANDLE_TYPE => {
                let resource = self.pop_file_where(|r| r.id == last.id).ok_or_else(|| {
                    ExecutionError::MissingResource {
                        resource_type: "streamable".to_owned(),
                    }
                })?;

                let metadata = SteamMetadata {
                    id: last.id,
                    type_: FILE_HANDLE_TYPE,
                };

                Ok((
                    Box::pin(ReaderStream::new(resource.file).map_err(|e| {
                        ExecutionError::IoError {
                            message: e.to_string(),
                            os_code: None,
                        }
                    })),
                    metadata,
                ))
            }
            _ => {
                unreachable!()
            }
        }
    }
}

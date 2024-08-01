extern crate log;

mod cli;
mod cloud;
mod config;
mod environment_resolver;
mod errors;
mod events;
mod module_version_resolver;
mod options;
mod shared;
mod tracing_setup;
mod utils;
mod worker;

use crate::cli::parse_cli_args;
use crate::environment_resolver::EnvironmentResolver;
use crate::environment_resolver::FileSystemEnvironmentResolver;
use crate::module_version_resolver::{CloudModuleVersionResolver, FileSystemModuleVersionResolver};
use crate::worker::Worker;
use cloud::agent::CloudAgentHandle;
use cloud::logger::start_cloud_logger;
use config::run_configuration_fetcher;
use config::{fetch_configuration, run_configuration_coordinator, Configuration};
use environment_resolver::{CloudEnvironmentResolver, SharedEnvironmentResolver};
use errors::WorkerError;
use events::{spawn_cloud_agent_observer, spawn_console_observer, WorkerEvent, WorkerEventSender};
use futures_util::Future;
use module_version_resolver::SharedModuleVersionResolver;
use options::{
    get_cloud_options, get_http_server_options, get_name, get_turbofuro_token, CloudOptions,
    HttpServerOptions,
};
use reqwest::Client;
use shared::WorkerStoppingReason;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, error, info};
use turbofuro_runtime::executor::{Environment, Global, GlobalBuilder};

use utils::shutdown::shutdown_signal;

async fn run_worker(
    config: Configuration,
    http_server_options: HttpServerOptions,
    global: Arc<Global>,
    module_version_resolver: SharedModuleVersionResolver,
    observer: WorkerEventSender,
    stop_signal: impl Future<Output = WorkerStoppingReason>,
) -> Result<(), WorkerError> {
    let mut worker = Worker::new(config, global, module_version_resolver, observer);

    worker.start_with_http_server(http_server_options).await?;

    let reason = stop_signal.await;

    worker.stop(reason).await;

    Ok(())
}

async fn load_config_from_file(file_path: PathBuf) -> Result<Configuration, WorkerError> {
    let mut file = File::open(file_path).await.unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).await.unwrap();
    let config: Configuration = serde_json::from_str(&contents).unwrap();

    debug!(
        "Configuration loaded from file:\n{}",
        serde_json::to_string_pretty(&config).unwrap()
    );

    Ok(config)
}

async fn get_environment(
    id: Option<&str>,
    environment_resolver: Arc<Mutex<dyn EnvironmentResolver>>,
) -> Result<Environment, WorkerError> {
    match id {
        Some(id) => environment_resolver.lock().await.get_environment(id).await,
        None => {
            info!("No environment specified in the config, using empty environment");
            Ok(Environment::default())
        }
    }
}

async fn run_worker_with_cloud_agent(
    cloud_options: CloudOptions,
    http_server_options: HttpServerOptions,
) -> Result<(), WorkerError> {
    let module_version_resolver = get_module_version_resolver(&cloud_options);
    let environment_resolver = get_environment_resolver(&cloud_options);

    // Flag to indicate when the worker should stop
    let mut shutdown_flag = false;

    // Load first configuration and environment
    let first_config = fetch_configuration(&cloud_options).await?;
    let config = Arc::new(Mutex::new(first_config.clone()));
    let environment = get_environment(
        first_config.environment_id.as_deref(),
        environment_resolver.clone(),
    )
    .await?;

    // Setup configuration updates
    let (config_id_update_sender, mut config_id_update_receiver) = broadcast::channel::<String>(3);
    let coordinator = run_configuration_coordinator(config.clone(), config_id_update_sender);

    // Run passive fetcher
    run_configuration_fetcher(cloud_options.clone(), coordinator.clone());

    // Cloud agent reloader
    let (cloud_reload_sender, mut cloud_reload_receiver) = broadcast::channel::<()>(3);

    // Worker events
    let (worker_event_sender, worker_event_receiver) = WorkerEvent::create_channel();

    // Setup global
    let global = Arc::new(
        GlobalBuilder::new()
            .environment(environment)
            .execution_logger(start_cloud_logger(cloud_options.clone()))
            .build(),
    );

    // Start cloud agent
    let cloud_agent_handle = CloudAgentHandle::new(
        cloud_options.clone(),
        global.clone(),
        module_version_resolver.clone(),
        environment_resolver.clone(),
        coordinator.clone(),
        cloud_reload_sender.clone(),
    )?;
    cloud_agent_handle.start().await;
    spawn_cloud_agent_observer(worker_event_receiver.clone(), cloud_agent_handle.clone());

    // Run worker indefinitely until a shutdown flag is raised
    loop {
        // Get current config
        let current_config = { config.lock().await.clone() };

        // Fetch and swap environment if needed
        if current_config.clone().environment_id.unwrap_or_default() != global.environment.load().id
        {
            let environment = get_environment(
                current_config.environment_id.as_deref(),
                environment_resolver.clone(),
            )
            .await?;
            global.environment.swap(Arc::new(environment));
        }

        run_worker(
            current_config,
            http_server_options.clone(),
            global.clone(),
            module_version_resolver.clone(),
            worker_event_sender.clone(),
            async {
                tokio::select! {
                    _ = config_id_update_receiver.recv() => {
                        // Config update received, we should restart the worker
                        info!("Configuration changed, stopping gracefully");
                        WorkerStoppingReason::ConfigurationChanged
                    }
                    _ = cloud_reload_receiver.recv() => {
                        info!("Worker restarting by cloud agent");
                        WorkerStoppingReason::RequestedByCloudAgent
                    }
                    _ = shutdown_signal() => {
                        info!("Signal received, stopping gracefully");
                        shutdown_flag = true;
                        WorkerStoppingReason::SignalReceived
                    }
                }
            },
        )
        .await?;

        if shutdown_flag {
            info!("Shutting down...");
            break;
        } else {
            info!("Restarting worker...");

            // Sleep for a 10 milliseconds before restarting to prevent a devastating restart loop
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    drop(cloud_agent_handle);

    Ok(())
}

pub fn get_module_version_resolver(cloud_options: &CloudOptions) -> SharedModuleVersionResolver {
    Arc::new(CloudModuleVersionResolver::new(
        Client::new(),
        cloud_options.cloud_url.clone(),
        cloud_options.token.clone(),
    ))
}

pub fn get_environment_resolver(cloud_options: &CloudOptions) -> SharedEnvironmentResolver {
    Arc::new(Mutex::new(CloudEnvironmentResolver::new(
        Client::new(),
        cloud_options.cloud_url.clone(),
        cloud_options.token.clone(),
    )))
}

static VERSION: &str = env!("CARGO_PKG_VERSION");

async fn startup() -> Result<(), WorkerError> {
    let args = parse_cli_args()?;
    let config_path_arg = args.config.clone();
    let http_server_options = get_http_server_options(args.clone())?;
    let turbofuro_token = get_turbofuro_token(args.clone());
    let name = get_name(args.clone());

    info!("Starting Turbofuro Worker {}", VERSION);
    match (turbofuro_token, config_path_arg) {
        (Some(token), None) => {
            let cloud_options = get_cloud_options(args.clone(), name, token);
            run_worker_with_cloud_agent(cloud_options, http_server_options).await
        }
        (None, Some(path)) => {
            // You can replace resolvers for quick local testing
            let module_version_resolver = Arc::new(FileSystemModuleVersionResolver {});
            let environment_resolver = Arc::new(Mutex::new(FileSystemEnvironmentResolver {}));

            // Load first configuration and environment
            let config = load_config_from_file(path).await?;
            let environment = get_environment(
                config.environment_id.as_deref(),
                environment_resolver.clone(),
            )
            .await?;

            let (worker_observer_sender, worker_observer_receiver) = WorkerEvent::create_channel();
            spawn_console_observer(worker_observer_receiver);

            let global = Arc::new(GlobalBuilder::new().environment(environment).build());

            Ok(run_worker(
                config,
                http_server_options,
                global,
                module_version_resolver,
                worker_observer_sender,
                async {
                    // The only way to reload/stop the worker is to send a signal
                    // Although murdering the process (kill -9 PID) is always an option, not recommended though
                    shutdown_signal().await;
                    WorkerStoppingReason::SignalReceived
                },
            )
            .await?)
        }
        (None, None) => Err(WorkerError::InvalidArguments {
            message: "Either token or config must be provided".to_string(),
        }),
        (Some(_), Some(_)) => Err(WorkerError::InvalidArguments {
            message: "Either token or config must be provided, but not both at the same time"
                .to_string(),
        }),
    }
}

#[tokio::main]
async fn main() {
    tracing_setup::init();

    match startup().await {
        Ok(_) => {
            info!("Worker finished successfully");
        }
        Err(e) => {
            error!("{}", e);
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module_version_resolver::FileSystemModuleVersionResolver;
    use axum::body::Bytes;
    use axum::response::Response;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        Router,
    };
    use futures_util::{SinkExt, StreamExt};
    use http_body_util::BodyExt;
    use hyper::Method;
    use serde_json::{json, Value};
    use std::future::IntoFuture;
    use std::net::{Ipv4Addr, SocketAddr};
    use tokio::time::timeout;
    use tokio_tungstenite::tungstenite;
    use tower::ServiceExt;
    use turbofuro_runtime::StorageValue;

    static TEST_CONFIG: &str = include_str!("../test_config.json");

    async fn get_test_app() -> Router {
        // Uncomment to enable logs
        tracing_setup::init();
        let module_version_resolver = Arc::new(FileSystemModuleVersionResolver {});
        let config: Configuration = serde_json::from_str(TEST_CONFIG).unwrap();

        let global = Arc::new(GlobalBuilder::new().build());

        let (worker_observer_sender, worker_observer_receiver) = WorkerEvent::create_channel();
        spawn_console_observer(worker_observer_receiver);

        let mut worker = Worker::new(
            config,
            global,
            module_version_resolver,
            worker_observer_sender,
        );

        worker.start().await.unwrap()
    }

    async fn get_body(response: Response) -> Bytes {
        response.into_body().collect().await.unwrap().to_bytes()
    }

    async fn start_test_server(app: Router) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(axum::serve(listener, app).into_future());
        addr
    }

    #[tokio::test]
    async fn not_found_where_non_existing_route() {
        let app = get_test_app().await;

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn errors_with_malformed_module() {
        let app = get_test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/malformed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn hello_world() {
        let app = get_test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/simple")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = get_body(response).await;
        assert_eq!(&body[..], b"Hello World!");
    }

    #[tokio::test]
    async fn echo_plain_text() {
        let app = get_test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/echo")
                    .header("content-type", "text/plain")
                    .body(Body::from("Hello World!"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = get_body(response).await;
        let body: StorageValue = serde_json::from_slice(&body).unwrap();

        assert_eq!(
            body,
            serde_json::from_value::<StorageValue>(json!(
                {
                    "query": {},
                    "method": "GET",
                    "headers": {
                      "content-type": "text/plain"
                    },
                    "params": {},
                    "cookies": {},
                    "path": "/echo",
                    "body": "Hello World!",
                    "version": "HTTP/1.1"
                  }
            ))
            .unwrap()
        );
    }

    #[tokio::test]
    async fn echo_json() {
        let app = get_test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/echo")
                    .header("content-type", "application/json")
                    .body(Body::from("{\"hello\": \"world\"}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = get_body(response).await;
        let body: StorageValue = serde_json::from_slice(&body).unwrap();

        assert_eq!(
            body,
            serde_json::from_value::<StorageValue>(json!(
                {
                    "query": {},
                    "method": "GET",
                    "headers": {
                      "content-type": "application/json"
                    },
                    "params": {},
                    "path": "/echo",
                    "body": {
                        "hello": "world"
                    },
                    "cookies": {},
                    "version": "HTTP/1.1"
                  }
            ))
            .unwrap()
        );
    }

    #[tokio::test]
    async fn conditional_triggered_endpoint() {
        let app = get_test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/conditional?name=John")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = get_body(response).await;
        let body: StorageValue = serde_json::from_slice(&body).unwrap();

        assert_eq!(
            body,
            serde_json::from_value::<StorageValue>(json!({ "msg": "Hello John!" })).unwrap()
        );
    }

    #[tokio::test]
    async fn conditional_not_triggered_endpoint() {
        let app = get_test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/conditional")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = get_body(response).await;
        let body: StorageValue = serde_json::from_slice(&body).unwrap();

        assert_eq!(
            body,
            serde_json::from_value::<StorageValue>(json!({ "msg": "Hello Stranger!" })).unwrap()
        );
    }

    #[tokio::test]
    async fn echo_websocket() {
        let app = get_test_app().await;
        let addr = start_test_server(app).await;

        let (mut socket, _response) =
            tokio_tungstenite::connect_async(format!("ws://{addr}/echo_ws"))
                .await
                .unwrap();

        socket
            .send(tungstenite::Message::text("foo"))
            .await
            .unwrap();

        let msg = match socket.next().await.unwrap().unwrap() {
            tungstenite::Message::Text(msg) => msg,
            other => panic!("expected a text message but got {other:?}"),
        };
        assert_eq!(msg, "foo");

        socket
            .send(tungstenite::Message::text("hello"))
            .await
            .unwrap();

        let msg = match socket.next().await.unwrap().unwrap() {
            tungstenite::Message::Text(msg) => msg,
            other => panic!("expected a text message but got {other:?}"),
        };
        assert_eq!(msg, "hello");
    }

    #[tokio::test]
    async fn test_http_to_ws() {
        let app = get_test_app().await;
        let addr = start_test_server(app).await;
        let (mut socket, _response) = timeout(
            Duration::from_secs(1),
            tokio_tungstenite::connect_async(format!("ws://{addr}/http-to-ws-socket")),
        )
        .await
        .unwrap()
        .unwrap();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            reqwest::get(format!("http://{addr}/http-to-ws"))
                .await
                .unwrap();
        });
        let msg = match timeout(Duration::from_secs(1), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            tungstenite::Message::Text(msg) => msg,
            other => panic!("expected a text message but got {other:?}"),
        };
        assert_eq!(msg, "message from http endpoint");
    }

    #[tokio::test]
    async fn test_loops_break_continue() {
        let app = get_test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/loops")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = get_body(response).await;
        let body: StorageValue = serde_json::from_slice(&body).unwrap();

        assert_eq!(
            body,
            serde_json::from_value::<StorageValue>(json!({ "continued": "yes", "x": 5 })).unwrap()
        );
    }

    #[tokio::test]
    async fn test_cors() {
        let app = get_test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/echo")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("access-control-allow-origin")
                .unwrap(),
            "*"
        );
    }

    #[tokio::test]
    async fn test_actor_demo() {
        let app = get_test_app().await;
        let addr = start_test_server(app).await;

        let response: Value = timeout(
            Duration::from_secs(1),
            reqwest::get(format!("http://{addr}/actor?name=John")),
        )
        .await
        .unwrap()
        .unwrap()
        .json()
        .await
        .unwrap();

        assert_eq!(
            response,
            json!(
                {
                    "message": "created",
                }
            )
        );

        let response: Value = timeout(
            Duration::from_secs(5),
            reqwest::get(format!("http://{addr}/actor?name=Maria")),
        )
        .await
        .unwrap()
        .unwrap()
        .json()
        .await
        .unwrap();

        assert_eq!(
            response,
            json!(
                {
                    "data": "John",
                }
            )
        );

        let response: Value = timeout(
            Duration::from_secs(5),
            reqwest::get(format!("http://{addr}/actor?name=James")),
        )
        .await
        .unwrap()
        .unwrap()
        .json()
        .await
        .unwrap();

        assert_eq!(
            response,
            json!(
                {
                    "data": "Maria",
                }
            )
        );
    }

    #[tokio::test]
    async fn test_sum() {
        let app = get_test_app().await;
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/sum")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = get_body(response).await;
        let body: StorageValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body,
            serde_json::from_value::<StorageValue>(json!({ "output": -7 })).unwrap()
        );
    }

    #[tokio::test]
    async fn test_late_response() {
        let app = get_test_app().await;
        let response = timeout(
            Duration::from_secs(5),
            app.oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/late")
                    .body(Body::empty())
                    .unwrap(),
            ),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = get_body(response).await;
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(text, "Hey after 1s");
    }

    #[tokio::test]
    async fn test_interval_with_ws() {
        let app = get_test_app().await;
        let addr = start_test_server(app).await;

        let now = tokio::time::Instant::now();
        let (mut socket, _response) =
            tokio_tungstenite::connect_async(format!("ws://{addr}/interval"))
                .await
                .unwrap();

        match socket.next().await.unwrap().unwrap() {
            tungstenite::Message::Text(msg) => {
                assert_eq!(msg, "Ping");
                assert!(now.elapsed().as_millis() >= 100);
            }
            other => panic!("expected a ping but got {other:?}"),
        };

        match socket.next().await.unwrap().unwrap() {
            tungstenite::Message::Text(msg) => {
                assert_eq!(msg, "Ping");
                assert!(now.elapsed().as_millis() >= 100);
            }
            other => panic!("expected a ping but got {other:?}"),
        };
    }
}

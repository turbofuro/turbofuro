extern crate log;

mod cli;
mod cloud_agent;
mod cloud_logger;
mod config;
mod environment_resolver;
mod module_version_resolver;
mod tracing_setup;
mod worker;

use crate::cli::parse_cli_args;
use crate::environment_resolver::FileSystemEnvironmentResolver;
use crate::module_version_resolver::{CloudModuleVersionResolver, FileSystemModuleVersionResolver};
use crate::worker::Worker;
use cloud_agent::CloudAgent;
use cloud_logger::start_cloud_logger;
use config::{
    fetch_configuration, run_configuration_coordinator, run_configuration_fetcher, Configuration,
};
use environment_resolver::{
    CloudEnvironmentResolver, EnvironmentResolver, SharedEnvironmentResolver,
};
use futures_util::Future;
use module_version_resolver::SharedModuleVersionResolver;
use reqwest::Client;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::sync::{broadcast, watch, Mutex};
use turbofuro_runtime::executor::{Global, GlobalBuilder};
use worker::WorkerError;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tracing::{debug, error, info, warn};

static BASE_DELAY: u64 = 50;
static MAX_ATTEMPTS_EXPONENT: u32 = 15;

async fn shutdown_signal(
    configuration_change: impl Future<Output = ()>,
    closing_flag: Arc<Mutex<bool>>,
) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");

        let mut flag = closing_flag.lock().await;
        *flag = true;
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;

        let mut flag = closing_flag.lock().await;
        *flag = true;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Signal received, starting graceful shutdown");
        },
        _ = terminate => {
            info!("Signal received, starting graceful shutdown");
        },
        _ = configuration_change => {
            info!("Configuration changed, starting graceful shutdown");
        },
    }
}

async fn start_worker(
    config: Configuration,
    environments_resolver: SharedEnvironmentResolver,
    http_server_options: HttpServerOptions,
    global: Arc<Global>,
    module_version_resolver: SharedModuleVersionResolver,
    shutdown_signal: impl Future<Output = ()>,
) -> Result<(), WorkerError> {
    let mut worker = Worker::new(
        config,
        global,
        module_version_resolver,
        environments_resolver,
    );

    worker.start_with_http_server(http_server_options).await?;

    shutdown_signal.await;

    worker.stop().await;

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

async fn setup_configuration_fetching(
    cloud_options: CloudOptions,
    http_server_options: HttpServerOptions,
) -> Result<(), WorkerError> {
    // Flag to indicate when the worker should stop
    let closing_flag = Arc::new(Mutex::new(false));

    // Fetch first configuration
    let first_config = fetch_configuration(&cloud_options).await?;
    let config = Arc::new(Mutex::new(first_config));

    // Setup configuration updates
    let (config_id_update_sender, mut config_id_update_receiver) = broadcast::channel::<String>(3);
    let coordinator = run_configuration_coordinator(config.clone(), config_id_update_sender);

    // Start passive fetcher
    run_configuration_fetcher(cloud_options.clone(), coordinator.clone());

    let worker_mutex = config.clone();
    let module_version_resolver = get_module_version_resolver(&cloud_options);
    let environment_resolver = get_environment_resolver(&cloud_options);

    // Setup global
    let global = Arc::new(
        GlobalBuilder::new()
            .execution_logger(start_cloud_logger(cloud_options.clone()))
            .build(),
    );
    let worker_global = Arc::clone(&global);

    // Start cloud agent
    let cloud_agent_environment_resolver: Arc<Mutex<dyn EnvironmentResolver>> =
        environment_resolver.clone();
    let cloud_agent_module_version_resolver = module_version_resolver.clone();
    let cloud_agent_cloud_options = cloud_options.clone();
    tokio::spawn(async move {
        let mut agent = CloudAgent {
            cloud_options: cloud_agent_cloud_options.clone(),
            global: global.clone(),
            environment_resolver: cloud_agent_environment_resolver.clone(),
            module_version_resolver: cloud_agent_module_version_resolver.clone(),
            configuration_coordinator: coordinator.clone(),
            name: cloud_agent_cloud_options.name.clone(),
        };

        // Let's try to connect with a exponential backoff
        let mut attempts = 1;
        loop {
            let mut failed = false;
            match agent.start().await {
                Ok(()) => {
                    attempts = 1;
                }
                Err(e) => {
                    error!("Cloud agent failed to connect: {:?}", e);
                    attempts += 1;
                    failed = true;
                }
            }
            agent = CloudAgent {
                cloud_options: cloud_agent_cloud_options.clone(),
                global: global.clone(),
                environment_resolver: cloud_agent_environment_resolver.clone(),
                module_version_resolver: cloud_agent_module_version_resolver.clone(),
                configuration_coordinator: coordinator.clone(),
                name: cloud_agent_cloud_options.name.clone(),
            };

            if failed {
                // Cap delay at ~1h
                let delay = Duration::from_millis(
                    2_u64.pow(attempts.min(MAX_ATTEMPTS_EXPONENT)) * BASE_DELAY,
                );
                warn!(
                    "Cloud agent failed to connect, waiting {} milliseconds before retry...",
                    delay.as_millis()
                );
                tokio::time::sleep(delay).await;
            }
        }
    });

    // Run worker indefinitely until a closing flag is raised
    loop {
        // Fetch current config
        let current_config = { worker_mutex.lock().await.clone() };

        start_worker(
            current_config,
            environment_resolver.clone(),
            http_server_options.clone(),
            worker_global.clone(),
            module_version_resolver.clone(),
            shutdown_signal(
                async {
                    config_id_update_receiver.recv().await.ok();
                },
                closing_flag.clone(),
            ),
        )
        .await?;

        let guard = closing_flag.lock().await;
        if *guard {
            info!("Shutting down...");
            break;
        } else {
            info!("Restarting worker...");

            // Sleep for a second before restarting to prevent a devastating restart loop
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

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

#[derive(Debug, Clone, PartialEq)]
pub struct HttpServerOptions {
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CloudOptions {
    pub cloud_url: String,
    pub operator_url: String,
    pub token: String,
    pub name: String,
}

// Remember to update the help message in cli.rs when changing those
static DEFAULT_CLOUD_URL: &str = "https://api.turbofuro.com";
static DEFAULT_OPERATOR_URL: &str = "wss://operator.turbofuro.com";
static TURBOFURO_TOKEN_ENV_NAME: &str = "TURBOFURO_TOKEN";
static TURBOFURO_CLOUD_URL_ENV_NAME: &str = "TURBOFURO_CLOUD_URL";
static TURBOFURO_OPERATOR_URL_ENV_NAME: &str = "TURBOFURO_OPERATOR_URL";
static PORT_ENV_NAME: &str = "PORT";
static NAME_ENV_NAME: &str = "NAME";

async fn startup() -> Result<(), WorkerError> {
    let args = parse_cli_args().map_err(|e| WorkerError::IncorrectParameters(e.to_string()))?;

    let turbofuro_token = args
        .token
        .or_else(|| std::env::var(TURBOFURO_TOKEN_ENV_NAME).ok());
    let config_path_env = args.config;
    let port = match args.port.ok_or_else(|| {
        std::env::var(PORT_ENV_NAME)
            .ok()
            .unwrap_or("4000".to_owned())
            .parse::<u16>()
            .map_err(|e| {
                WorkerError::IncorrectEnvironmentVariable(PORT_ENV_NAME.into(), e.to_string())
            })
    }) {
        Ok(p) => p,
        Err(p) => p?,
    };

    let name = args
        .name
        .or_else(|| std::env::var(NAME_ENV_NAME).ok())
        .or_else(|| {
            {
                hostname::get()
                    .map(|s| s.to_string_lossy().into_owned())
                    .ok()
            }
            .map(|s| s.chars().take(200).collect())
        })
        .unwrap_or("Unknown".to_owned());

    let http_server_options = HttpServerOptions { port };

    info!("Starting Turbofuro Worker {}", env!("CARGO_PKG_VERSION"));
    match (turbofuro_token, config_path_env) {
        (Some(token), None) => {
            let cloud_options = CloudOptions {
                cloud_url: args
                    .cloud_url
                    .or_else(|| std::env::var(TURBOFURO_CLOUD_URL_ENV_NAME).ok())
                    .unwrap_or(DEFAULT_CLOUD_URL.to_owned()),
                operator_url: args
                    .operator_url
                    .or_else(|| std::env::var(TURBOFURO_OPERATOR_URL_ENV_NAME).ok())
                    .unwrap_or(DEFAULT_OPERATOR_URL.to_owned()),
                token,
                name,
            };

            if cloud_options.cloud_url != DEFAULT_CLOUD_URL {
                info!(
                    "Using custom cloud URL: {}",
                    cloud_options.cloud_url.clone()
                );
            }

            if cloud_options.operator_url != DEFAULT_OPERATOR_URL {
                info!(
                    "Using custom operator URL: {}",
                    cloud_options.operator_url.clone()
                );
            }

            setup_configuration_fetching(cloud_options, http_server_options).await
        }
        (None, Some(path)) => {
            let config = load_config_from_file(path).await?;
            let (_tx, mut rx) = watch::channel::<String>(config.id.clone());
            let closing_flag = Arc::new(Mutex::new(false));

            // You can replace resolvers for quick local testing
            let module_version_resolver = Arc::new(FileSystemModuleVersionResolver {});
            let environment_resolver = Arc::new(Mutex::new(FileSystemEnvironmentResolver {}));
            let global = Arc::new(GlobalBuilder::new().build());

            Ok(start_worker(
                config,
                environment_resolver,
                http_server_options,
                global,
                module_version_resolver,
                shutdown_signal(
                    async {
                        rx.changed().await.ok();
                    },
                    closing_flag,
                ),
            )
            .await?)
        }
        (None, None) => Err(WorkerError::IncorrectParameters(
            "Either token or config must be provided".to_string(),
        )),
        (Some(_), Some(_)) => Err(WorkerError::IncorrectParameters(
            "Either token or config must be provided, but not both at the same time".to_string(),
        )),
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
        let environment_resolver = Arc::new(Mutex::new(FileSystemEnvironmentResolver {}));
        let config: Configuration = serde_json::from_str(TEST_CONFIG).unwrap();

        let global = Arc::new(GlobalBuilder::new().build());

        let mut worker = Worker::new(
            config,
            global,
            module_version_resolver,
            environment_resolver,
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
                    "path": "/echo",
                    "body": "Hello World!"
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
                    }
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

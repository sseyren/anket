mod models;
mod utils;
mod views;

use crate::models::message;

use axum::response::IntoResponse;
use axum::{http, middleware, response, routing};
use rust_embed::RustEmbed;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::{
    self, signal,
    sync::{mpsc, Mutex},
};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub const SESSION_KEY: &str = "anket_session";
pub const SESSION_DURATION: cookie::time::Duration = cookie::time::Duration::weeks(52);

#[derive(RustEmbed)]
#[folder = "../frontend/dist/"]
struct StaticFiles;

async fn static_handler(uri: http::uri::Uri, config: AppConfig) -> response::Response {
    let path = uri.path().trim_start_matches('/');
    if path == "globals.js" {
        let hostd = config.host;
        let globals_js_content = format!(
            "\
var ANKET_HOST = \"{}\";
var ANKET_SECURE = {};
",
            hostd.host(),
            hostd.secure
        );
        return (
            [(
                http::header::CONTENT_TYPE,
                mime_guess::mime::APPLICATION_JAVASCRIPT.as_ref(),
            )],
            globals_js_content,
        )
            .into_response();
    }
    let (mime, content) = match StaticFiles::get(path) {
        Some(content) => (mime_guess::from_path(path).first_or_octet_stream(), content),
        None => (
            mime_guess::mime::TEXT_HTML,
            StaticFiles::get("index.html")
                .expect("at least an index.html file expected in StaticFiles"),
        ),
    };
    ([(http::header::CONTENT_TYPE, mime.as_ref())], content.data).into_response()
}

#[derive(Clone)]
pub struct AppState {
    config: Arc<AppConfig>,
    users: Arc<Mutex<models::Users>>,
    polls: mpsc::Sender<message::PollOp>,
}

impl AppState {
    fn init(config: AppConfig) -> AppState {
        let users = models::Users::init();
        let (polls_sender, _) = models::Polls::init();
        AppState {
            config: Arc::new(config),
            users: Arc::new(Mutex::new(users)),
            polls: polls_sender,
        }
    }
}

#[derive(Clone, Debug)]
struct HostDetails {
    secure: bool,
    hostname: String,
    path: String,
    port: Option<u16>,
}

impl HostDetails {
    fn host(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{}", self.hostname, port),
            None => self.hostname.clone(),
        }
    }
}

#[derive(Clone, Debug)]
struct AppConfig {
    host: HostDetails,
    bind_addr: SocketAddr,
    user_ip_lookup: bool,
}

fn get_config() -> AppConfig {
    let root_uri = std::env::var("ANKET_ROOT")
        .expect("ANKET_ROOT env variable must be set")
        .parse::<http::uri::Uri>()
        .expect("ANKET_ROOT is not a valid URI");

    let secure = if root_uri.scheme() == Some(&http::uri::Scheme::HTTPS) {
        true
    } else if root_uri.scheme() == Some(&http::uri::Scheme::HTTP) {
        false
    } else {
        panic!("ANKET_ROOT URI does not have a valid scheme");
    };

    let mut hostname = "".to_string();
    root_uri
        .host()
        .expect("ANKET_ROOT URI does not have a valid hostname")
        .clone_into(&mut hostname);

    let mut path = "/".to_string();
    root_uri.path().clone_into(&mut path);

    let port = root_uri.port_u16();

    let bind_addr = std::env::var("ANKET_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:3000".into())
        .parse::<SocketAddr>()
        .expect("ANKET_LISTEN is not a valid socket address");

    let user_ip_lookup = match std::env::var("ANKET_IP_LOOKUP")
        .unwrap_or_else(|_| "0".into())
        .parse::<usize>()
        .expect("ANKET_IP_LOOKUP is not valid; it can be 0 or 1")
    {
        0 => false,
        1 => true,
        _ => panic!("ANKET_IP_LOOKUP is not valid; it can be 0 or 1"),
    };

    AppConfig {
        host: HostDetails {
            secure,
            hostname,
            path,
            port,
        },
        bind_addr,
        user_ip_lookup,
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("signal received, starting graceful shutdown");
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_env("ANKET_LOG")
                .unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let app_config = get_config();
    let app_state = AppState::init(app_config.clone());

    let api_routes = routing::Router::new()
        .route("/poll", routing::post(views::create_poll))
        .route("/poll/:id", routing::get(views::poll_events))
        .route_layer(middleware::from_fn_with_state(
            app_state.clone(),
            views::identify_user,
        ))
        .with_state(app_state);

    let routes = routing::Router::new().nest("/api", api_routes).fallback({
        let app_config = app_config.clone();
        move |uri| static_handler(uri, app_config)
    });

    info!("started on {}", &app_config.bind_addr);
    axum::Server::bind(&app_config.bind_addr)
        .serve(routes.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

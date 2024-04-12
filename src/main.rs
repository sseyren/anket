mod models;
mod utils;
mod views;

use axum::{http, middleware, routing};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::{self, signal};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub const SESSION_KEY: &str = "anket_session";
pub const SESSION_DURATION: cookie::time::Duration = cookie::time::Duration::weeks(52);

#[derive(Clone)]
pub struct AppState {
    config: Arc<AppConfig>,
    polls: Arc<Mutex<models::Polls>>,
    templates: minijinja::Environment<'static>,
}

impl AppState {
    fn init(config: AppConfig) -> Self {
        let polls = models::Polls::new();
        let templates = {
            let mut env = minijinja::Environment::new();
            minijinja_embed::load_templates!(&mut env);
            env
        };

        Self {
            config: Arc::new(config),
            polls: Arc::new(Mutex::new(polls)),
            templates,
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

// TODO burada yer alan bazi seylere artik ihtiyacimiz kalmamis olabilir
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

    let routes = routing::Router::new()
        .route(
            "/p",
            routing::get(views::poll_index).post(views::create_poll),
        )
        .route("/p/:id", routing::get(views::get_poll))
        .route("/p/:id/ws", routing::get(views::join_poll))
        .route_layer(middleware::from_fn_with_state(
            app_state.clone(),
            views::identify_user,
        ))
        .nest("/assets", views::assets_router(app_state.clone()))
        .with_state(app_state);

    // TODO add handlers to custom errors like; 404, Failed to deserialize form body

    info!("started on {}", &app_config.bind_addr);
    axum::Server::bind(&app_config.bind_addr)
        .serve(routes.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

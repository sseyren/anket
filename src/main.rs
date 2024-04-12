mod models;
mod utils;
mod views;

use axum::{middleware, routing};
use std::borrow::Borrow;
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
            polls: polls,
            templates,
        }
    }
}

#[derive(Clone, Debug)]
struct AppConfig {
    bind_addr: SocketAddr,
    secure: bool,
}

fn get_config() -> AppConfig {
    let bind_addr = std::env::var("ANKET_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:3000".into())
        .parse::<SocketAddr>()
        .expect("ANKET_LISTEN is not a valid socket address");

    let secure = match std::env::var("ANKET_SECURE")
        .unwrap_or_else(|_| "0".into())
        .borrow()
    {
        "0" => false,
        "1" => true,
        _ => panic!("ANKET_SECURE can be 0 or 1"),
    };

    AppConfig { bind_addr, secure }
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
        .with_state(app_state)
        .route("/", routing::get(views::anket_index))
        // TODO remove this and use tower-http layer
        .route(
            "/p/",
            routing::get(|| async { axum::response::Redirect::temporary("/p") }),
        );

    // TODO add handlers to custom errors like; 404, Failed to deserialize form body

    info!("started on {}", &app_config.bind_addr);
    axum::Server::bind(&app_config.bind_addr)
        .serve(routes.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

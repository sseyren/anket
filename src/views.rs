use crate::{models, utils, AppState, SESSION_DURATION, SESSION_KEY};

use axum::{
    extract::{rejection, ws, ConnectInfo, Extension, Path, State},
    http::{header, Request, StatusCode},
    middleware,
    response::{Html, IntoResponse, Redirect, Response},
    routing, Form,
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use futures_util::{sink::SinkExt, stream::StreamExt};
use minijinja::context;
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    str::FromStr,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;
use uuid::Uuid;

// TODO transform this into tower middleware
pub async fn identify_user<B>(
    ConnectInfo(socket_addr): ConnectInfo<SocketAddr>,
    headers: header::HeaderMap,
    cookies: CookieJar,
    mut request: Request<B>,
    next: middleware::Next<B>,
) -> Response {
    let user = {
        // TODO improve user IP deriving process
        let ip = match headers.get("X-Forwarded-For") {
            Some(header) => match utils::forwarded_header_ip(header) {
                Some(ip) => ip,
                None => socket_addr.ip(),
            },
            None => socket_addr.ip(),
        };
        let id = match cookies.get(SESSION_KEY) {
            Some(cookie) => Uuid::from_str(cookie.value()).ok(),
            None => None,
        };
        models::UserDetails { ip, id }
    };
    request.extensions_mut().insert(user);
    next.run(request).await
}

pub fn assets_router(state: AppState) -> routing::Router<AppState> {
    routing::Router::new()
        .route(
            "/anket.css",
            routing::get(|State(state): State<AppState>| async move {
                (
                    [(header::CONTENT_TYPE, "text/css")],
                    state
                        .templates
                        .get_template("anket.css")
                        .unwrap()
                        .render(context!())
                        .unwrap(),
                )
            }),
        )
        .route(
            "/poll.js",
            routing::get(|State(state): State<AppState>| async move {
                (
                    [(header::CONTENT_TYPE, "text/javascript")],
                    state
                        .templates
                        .get_template("poll.js")
                        .unwrap()
                        .render(context!())
                        .unwrap(),
                )
            }),
        )
        .with_state(state)
}

pub async fn handler_404(State(state): State<AppState>) -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(
            state
                .templates
                .get_template("404.jinja")
                .unwrap()
                .render(context!())
                .unwrap(),
        ),
    )
        .into_response()
}

pub async fn anket_index() -> Response {
    // TODO make an actual index page
    Redirect::temporary("/p").into_response()
}

pub async fn poll_index(State(state): State<AppState>) -> Response {
    Html(
        state
            .templates
            .get_template("poll-form.jinja")
            .unwrap()
            .render(context!())
            .unwrap(),
    )
    .into_response()
}

fn poll_cookie(user_id: &Uuid, poll_id: &str, secure: bool) -> Cookie<'static> {
    Cookie::build(SESSION_KEY, user_id.to_string())
        .max_age(SESSION_DURATION)
        .http_only(false)
        .path(format!("/p/{}", poll_id))
        .secure(secure)
        .finish()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CreatePollReq {
    #[serde(flatten)]
    settings: models::PollSettings,
}

pub async fn create_poll(
    State(state): State<AppState>,
    Extension(user): Extension<models::UserDetails>,
    cookies: CookieJar,
    form: Result<Form<CreatePollReq>, rejection::FormRejection>,
) -> Response {
    let form_with_err = |msg: &str| {
        (
            StatusCode::BAD_REQUEST,
            Html(
                state
                    .templates
                    .get_template("poll-form.jinja")
                    .unwrap()
                    .render(context!(error => msg))
                    .unwrap(),
            ),
        )
            .into_response()
    };

    if let Err(err) = form {
        return form_with_err(&err.to_string());
    }

    let Form(form) = form.expect("we checked that this form is valid");
    if form.settings.title.len() < 3 {
        return form_with_err("Poll title must be at least 3 characters long.");
    }

    let (user_id, poll) = state.polls.lock().unwrap().add_poll(form.settings, user);
    let poll_id = poll.lock().unwrap().get_id().to_owned();
    let cookies = cookies.add(poll_cookie(&user_id, &poll_id, state.config.secure));

    (cookies, Redirect::to(&format!("/p/{}", poll_id))).into_response()
}

pub async fn get_poll(State(state): State<AppState>, Path(poll_id): Path<String>) -> Response {
    match state.polls.lock().unwrap().get_poll(&poll_id) {
        Some(_) => Html(
            state
                .templates
                .get_template("poll.jinja")
                .unwrap()
                .render(context!())
                .unwrap(),
        )
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Html(
                state
                    .templates
                    .get_template("404.jinja")
                    .unwrap()
                    .render(
                        context!(detail => "The poll you are looking for may have been closed."),
                    )
                    .unwrap(),
            ),
        )
            .into_response(),
    }
}

pub async fn join_poll(
    State(state): State<AppState>,
    Extension(user): Extension<models::UserDetails>,
    Path(poll_id): Path<String>,
    ws: ws::WebSocketUpgrade,
) -> Response {
    let poll = state.polls.lock().unwrap().get_poll(&poll_id);
    match poll {
        Some(poll) => {
            let (user_sender, user_receiver) = mpsc::unbounded_channel();
            let user_id = poll.lock().unwrap().join(user, user_sender);

            // TODO consider using `ws.on_failed_upgrade`?
            let mut response =
                ws.on_upgrade(move |socket| events_handler(socket, user_id, poll, user_receiver));
            response.headers_mut().append(
                header::SET_COOKIE,
                poll_cookie(&user_id, &poll_id, state.config.secure)
                    .encoded()
                    .to_string()
                    .parse()
                    .expect("nothing to fail; cookie details doesn't have anything user provided"),
            );
            response
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", content = "content")]
pub enum UserMessage {
    AddItem { text: String },
    VoteItem { item_id: usize, vote: isize },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", content = "content")]
pub enum UserResponse {
    ActionResponse(String),
    PollStateUpdate(models::PollState),
}

impl From<UserResponse> for ws::Message {
    fn from(val: UserResponse) -> Self {
        ws::Message::Text(serde_json::to_string(&val).expect("PollState should serialize"))
    }
}

fn websocket_worker(
    mut sender: futures_util::stream::SplitSink<ws::WebSocket, ws::Message>,
) -> (
    tokio::task::JoinHandle<Result<(), axum::Error>>,
    mpsc::UnboundedSender<ws::Message>,
) {
    let (task_sender, mut task_receiver) = mpsc::unbounded_channel();

    let task = tokio::spawn(async move {
        while let Some(message) = task_receiver.recv().await {
            sender.send(message).await?
        }
        Ok(())
    });

    (task, task_sender)
}

async fn events_handler(
    socket: ws::WebSocket,
    user_id: Uuid,
    poll: Arc<Mutex<models::Poll>>,
    mut user_receiver: mpsc::UnboundedReceiver<models::PollState>,
) {
    let (ws_sender, mut ws_receiver) = socket.split();
    let (ws_task, ws_sender) = websocket_worker(ws_sender);

    let poll_task = {
        let ws_sender = ws_sender.clone();
        tokio::spawn(async move {
            while let Some(state) = user_receiver.recv().await {
                let msg = UserResponse::PollStateUpdate(state);
                let send = ws_sender.send(msg.into());
                if send.is_err() {
                    break;
                }
            }
        })
    };

    let user_task = tokio::spawn(async move {
        while let Some(wsmsg) = ws_receiver.next().await {
            if let Ok(ws::Message::Text(text)) = wsmsg {
                let response = match serde_json::from_str::<UserMessage>(&text) {
                    Ok(msg) => match msg {
                        UserMessage::AddItem { text } => {
                            if text.is_empty() {
                                Some(UserResponse::ActionResponse(
                                    "Poll item text cannot be empty.".to_string(),
                                ))
                            } else {
                                poll.lock()
                                    .unwrap()
                                    .add_item(user_id, text)
                                    .err()
                                    .map(|err| UserResponse::ActionResponse(err.to_string()))
                            }
                        }
                        UserMessage::VoteItem { item_id, vote } => poll
                            .lock()
                            .unwrap()
                            .vote_item(user_id, item_id, vote)
                            .err()
                            .map(|err| UserResponse::ActionResponse(err.to_string())),
                    },
                    Err(_) => Some(UserResponse::ActionResponse(
                        "Failed to deserialize client message.".to_string(),
                    )),
                };
                if let Some(resp) = response {
                    if ws_sender.send(resp.into()).is_err() {
                        break;
                    }
                }
            } else if let Ok(ws::Message::Close(_)) = wsmsg {
                // client disconnected
                break;
            } else if wsmsg.is_err() {
                // client disconnected
                break;
            }
        }
    });

    let poll_handle = poll_task.abort_handle();
    let user_handle = user_task.abort_handle();
    let ws_handle = ws_task.abort_handle();

    tokio::select! {
        _ = poll_task => {
            user_handle.abort();
            ws_handle.abort();
        }
        _ = user_task => {
            poll_handle.abort();
            ws_handle.abort();
        }
        _ = ws_task => {
            poll_handle.abort();
            user_handle.abort();
        }
    }
}

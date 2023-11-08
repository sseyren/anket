use crate::{
    models::{self, message},
    utils, AppState, SESSION_DURATION, SESSION_KEY,
};

use message::ToInPollOp;
use models::PollOpSender;

use axum::{
    body,
    extract::{ws, ConnectInfo, Extension, Path, State},
    http::{
        header::{self, HeaderMap},
        Request, StatusCode,
    },
    middleware,
    response::{IntoResponse, Response},
    Json,
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use futures_util::{sink::SinkExt, stream::StreamExt};
use std::net::SocketAddr;
use tokio::sync::mpsc;
use uuid::Uuid;

pub async fn identify_user<B>(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
    mut request: Request<B>,
    next: middleware::Next<B>,
) -> Response {
    let user_ip = match headers.get("X-Forwarded-For") {
        Some(header) => match utils::forwarded_header_ip(header) {
            Some(ip) => ip,
            None => addr.ip(),
        },
        None => addr.ip(),
    };

    let user = {
        let mut mutmap = state.users.lock().await;
        let user_by_session = match jar.get(SESSION_KEY) {
            Some(cookie) => match mutmap.get_user_by_session(cookie.value()) {
                Some(u) => Some(u.clone()),
                None => None,
            },
            None => None,
        };
        user_by_session.unwrap_or_else(|| match state.config.user_ip_lookup {
            true => match mutmap.get_user_by_ip(&user_ip) {
                Some(u) => u.clone(),
                None => mutmap.new_user(user_ip).clone(),
            },
            false => mutmap.new_user(user_ip).clone(),
        })
    };
    request.extensions_mut().insert(user.clone());

    let mut response = next.run(request).await;

    let req_cookie = jar.get(SESSION_KEY);
    if req_cookie.is_none() || req_cookie.unwrap().value() != &user.session {
        let new_cookie = Cookie::build(SESSION_KEY, user.session.clone())
            .max_age(SESSION_DURATION)
            .http_only(false)
            .path(state.config.host.path.clone())
            .secure(state.config.host.secure)
            .finish();

        let jar = jar.add(new_cookie);
        for cookie in jar.iter() {
            if let Ok(header_value) = cookie.encoded().to_string().parse() {
                response
                    .headers_mut()
                    .append(header::SET_COOKIE, header_value);
            }
        }
    }

    response
}

pub async fn create_poll(
    State(state): State<AppState>,
    Extension(user): Extension<models::User>,
    Json(msg): Json<anket_shared::CreatePollReq>,
) -> Response {
    if msg.title.is_empty() {
        StatusCode::BAD_REQUEST.into_response()
    } else {
        let poll_id = state.polls.create_poll(user.id, msg.title.clone()).await;
        Json(anket_shared::CreatePollResp {
            id: poll_id,
            title: msg.title.clone(),
        })
        .into_response()
    }
}

pub async fn poll_events(
    State(state): State<AppState>,
    Extension(user): Extension<models::User>,
    Path(poll_id): Path<String>,
    ws: ws::WebSocketUpgrade,
) -> Response {
    let (user_sender, user_receiver) = mpsc::unbounded_channel();
    let poll_sender = state.polls.join_poll(user.id, poll_id, user_sender).await;
    match poll_sender {
        Some(poll_sender) => {
            ws.on_upgrade(move |socket| events_handler(socket, user.id, poll_sender, user_receiver))
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(body::boxed(body::Empty::new()))
            .expect("should be able to create empty response"),
    }
}

async fn events_handler(
    socket: ws::WebSocket,
    user_id: Uuid,
    poll_sender: mpsc::Sender<message::InPollOp>,
    mut user_receiver: mpsc::UnboundedReceiver<anket_shared::PollState>,
) {
    let (mut ws_sender, mut ws_receiver) = socket.split();

    let poll_task = tokio::spawn(async move {
        while let Some(msg) = user_receiver.recv().await {
            let wsmsg =
                ws::Message::Text(serde_json::to_string(&msg).expect("PollState should serialize"));
            if ws_sender.send(wsmsg).await.is_err() {
                break;
            }
        }
    });
    let user_task = tokio::spawn(async move {
        while let Some(wsmsg) = ws_receiver.next().await {
            if let Ok(ws::Message::Text(text)) = wsmsg {
                if let Ok(msg) = serde_json::from_str::<anket_shared::Message>(&text) {
                    if poll_sender.send(msg.to_poll(user_id)).await.is_err() {
                        // poll ended
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

    tokio::select! {
        _ = poll_task => {
            user_handle.abort();
        }
        _ = user_task => {
            poll_handle.abort();
        }
    }
}

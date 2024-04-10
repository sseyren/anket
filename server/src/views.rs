use crate::{models, utils, AppState, SESSION_DURATION, SESSION_KEY};

use axum::{
    body,
    extract::{ws, ConnectInfo, Extension, Path, State},
    http::{header::HeaderMap, Request, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    Form,
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use futures_util::{sink::SinkExt, stream::StreamExt};
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
    State(state): State<AppState>,
    ConnectInfo(socket_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
    mut request: Request<B>,
    next: middleware::Next<B>,
) -> Response {
    let user = {
        // TODO bu header'in baska varyasyonlari da var mi?
        let ip = match headers.get("X-Forwarded-For") {
            Some(header) => match utils::forwarded_header_ip(header) {
                Some(ip) => ip,
                None => socket_addr.ip(),
            },
            None => socket_addr.ip(),
        };
        let id = match jar.get(SESSION_KEY) {
            Some(cookie) => Uuid::from_str(cookie.value()).ok(),
            None => None,
        };
        models::UserDetails { ip, id }
    };

    request.extensions_mut().insert(user);
    next.run(request).await
}

const template_form: &str = r#"
<form method="POST">
    <input type="text" name="title" />
    <select name="user_lookup_method">
        <option value="SessionBased">Session Based</option>
        <option value="IPBased">IP Based</option>
    </select>

    <input type="submit" value="Create Poll">
</form>
"#;

pub async fn poll_index() -> Response {
    axum::response::Html(template_form).into_response()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CreatePollReq {
    #[serde(flatten)]
    settings: models::PollSettings,
}

pub async fn create_poll(
    State(state): State<AppState>,
    Extension(user): Extension<models::UserDetails>,
    cookie_jar: CookieJar,
    Form(form): Form<CreatePollReq>,
) -> Response {
    // TODO return an actual response for this
    if form.settings.title.len() < 2 {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let (user_id, poll) = state.polls.lock().unwrap().add_poll(form.settings, user);
    let poll_id = poll.lock().unwrap().get_id().to_owned();

    let cookie_jar = cookie_jar.add(
        Cookie::build(SESSION_KEY, user_id.to_string())
            .max_age(SESSION_DURATION)
            .http_only(false)
            .path(format!("{}poll/{}", state.config.host.path, poll_id)) // TODO bug when using / as path value // TODO use const vars for path
            .secure(state.config.host.secure)
            .finish(),
    );

    (
        cookie_jar,
        axum::response::Html(format!(r#"<h1>Poll created. Poll ID: {}</h1>"#, poll_id)),
    )
        .into_response()
}

// TODO don't forget to return cookie in response
pub async fn poll_events(
    State(state): State<AppState>,
    Extension(user): Extension<models::UserDetails>,
    Path(poll_id): Path<String>,
    ws: ws::WebSocketUpgrade,
) -> Response {
    let (user_sender, user_receiver) = mpsc::unbounded_channel();

    let poll = state.polls.lock().unwrap().get_poll(&poll_id);
    match poll {
        Some(poll) => {
            // TODO remove unwrap
            let user_id = poll.lock().unwrap().join(user, user_sender).unwrap();
            ws.on_upgrade(move |socket| events_handler(socket, user_id, poll, user_receiver))
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
    poll: Arc<Mutex<crate::models::Poll>>,
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
                    match msg {
                        anket_shared::Message::AddItem { text } => {
                            poll.lock().unwrap().add_item(user_id, text);
                        }
                        anket_shared::Message::VoteItem { item_id, vote } => {
                            poll.lock().unwrap().vote_item(user_id, item_id, vote);
                        }
                    }
                } else {
                    // TODO we need to inform user
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

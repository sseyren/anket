use anket_shared::Message as SharedMsg;
use anket_shared::{CreatePollReq, CreatePollResp, ItemState, PollState};

use futures::{stream::SplitSink, SinkExt, StreamExt};
use gloo_net::http::Request;
use gloo_net::websocket::{futures::WebSocket, Message};
use wasm_bindgen::{prelude::wasm_bindgen, JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlInputElement;
use yew::prelude::*;
use yew_router::prelude::*;

#[wasm_bindgen]
extern "C" {
    static ANKET_HOST: JsValue;
    static ANKET_SECURE: JsValue;
}

enum Scheme {
    WS,
    #[allow(dead_code)]
    HTTP,
}

fn root_endpoint(scheme: Scheme) -> String {
    let host = ANKET_HOST
        .clone()
        .as_string()
        .expect("this expected to be defined in globals.js");
    let secure = ANKET_SECURE
        .clone()
        .as_bool()
        .expect("this expected to be defined in globals.js");
    let scheme = match scheme {
        Scheme::WS => {
            if secure {
                "wss"
            } else {
                "ws"
            }
        }
        Scheme::HTTP => {
            if secure {
                "https"
            } else {
                "http"
            }
        }
    };
    format!("{scheme}://{host}/api")
}

#[derive(Clone, Properties, PartialEq)]
struct ItemProps {
    state: ItemState,
    onclick: Callback<(usize, isize)>, // item id, vote value
}

#[function_component(PollItem)]
fn poll_item_fn(props: &ItemProps) -> Html {
    let up_click = {
        let props = props.clone();
        let value: isize = if props.state.user_vote == 1 { 0 } else { 1 };
        Callback::from(move |_| props.onclick.emit((props.state.id, value)))
    };
    let down_click = {
        let props = props.clone();
        let value: isize = if props.state.user_vote == -1 { 0 } else { -1 };
        Callback::from(move |_| props.onclick.emit((props.state.id, value)))
    };
    html! {
        <div class="option-card">
              <div class="option-vote">
                    <button class="pure-button option-vote-button" onclick={ up_click }>
                        { if props.state.user_vote == 1 {"⬆"} else {"⇧"} }
                    </button>
                    <div class="option-score">{ props.state.score }</div>
                    <button class="pure-button option-vote-button" onclick={ down_click }>
                        { if props.state.user_vote == -1 {"⬇"} else {"⇩"} }
                    </button>
              </div>
              <div class="option-content">{ &props.state.text }</div>
        </div>
    }
}

#[derive(Clone, Properties, PartialEq)]
struct PollProps {
    id: String,
}

#[function_component(Poll)]
fn poll(PollProps { id }: &PollProps) -> Html {
    let state = use_state(|| PollState {
        poll_title: "".to_string(),
        top_items: vec![],
        latest_items: vec![],
        user_items: vec![],
    });
    let ws_sender = use_mut_ref(|| None::<SplitSink<WebSocket, Message>>);
    let item_input = use_node_ref();

    let add_item_callback = {
        let ws_sender = ws_sender.clone();
        let item_input = item_input.clone();
        Callback::from(move |event: SubmitEvent| {
            event.prevent_default();
            let input_elem = item_input.cast::<HtmlInputElement>().unwrap();
            let item_text = input_elem.value();
            input_elem.set_value("");

            if !item_text.is_empty() {
                let ws_sender = ws_sender.clone();
                spawn_local(async move {
                    let msg = Message::Text(
                        serde_json::to_string(&SharedMsg::AddItem { text: item_text }).unwrap(),
                    );
                    let mut binding = ws_sender.borrow_mut();
                    let sender = binding.as_mut().unwrap();
                    sender.send(msg).await.unwrap();
                });
            }
        })
    };

    let vote_buttons_callback = {
        let ws_sender = ws_sender.clone();
        Callback::from(move |(item_id, vote)| {
            let ws_sender = ws_sender.clone();
            spawn_local(async move {
                let msg = Message::Text(
                    serde_json::to_string(&SharedMsg::VoteItem { item_id, vote }).unwrap(),
                );
                let mut binding = ws_sender.borrow_mut();
                let sender = binding.as_mut().unwrap();
                sender.send(msg).await.unwrap();
            });
        })
    };

    {
        let poll_id = id.clone();
        let state = state.clone();
        let ws_sender = ws_sender.clone();
        use_effect_with((), move |_| {
            let root = root_endpoint(Scheme::WS);
            let ws = WebSocket::open(&format!("{root}/poll/{poll_id}")).unwrap();
            let (sender, mut receiver) = ws.split();
            *ws_sender.borrow_mut() = Some(sender);
            spawn_local(async move {
                loop {
                    match receiver.next().await {
                        Some(wsmsg) => match wsmsg {
                            Ok(msg) => {
                                if let Message::Text(text) = msg {
                                    let new_state: PollState = serde_json::from_str(&text).unwrap();
                                    state.set(new_state);
                                }
                            }
                            Err(_) => break,
                        },
                        None => break,
                    }
                }
            });
            || ()
        })
    }

    let render_item = |i: &ItemState| {
        html! {
            <PollItem key={i.id} state={i.clone()} onclick={vote_buttons_callback.clone()} />
        }
    };

    html! {
        <div>
            <div class="pure-g">
                <div class="pure-u-1-24"></div>
                <div class="pure-u-22-24">
                    <div class="pure-g">
                        <div class="pure-u-1">
                            <h1>{ format!("Poll: {}", state.poll_title ) }</h1>
                            <form class="pure-form" onsubmit={add_item_callback}>
                                <fieldset>
                                    <legend>{"Create an option for this poll"}</legend>
                                    <input
                                      class="pure-input-3-4"
                                      type="text"
                                      placeholder="Option text"
                                      ref={item_input.clone()}
                                    />
                                    <button
                                      type="submit"
                                      class="pure-button pure-input-1-4 pure-button-primary"
                                    >
                                      {"Create"}
                                    </button>
                                </fieldset>
                            </form>
                        </div>
                    </div>
                    <div class="pure-g">
                        <div class="pure-u-1-3">
                            <h2 class="text-center">{"Top Voted Items"}</h2>
                            { for state.top_items.iter().map(render_item) }
                        </div>
                        <div class="pure-u-1-3">
                            <h2 class="text-center">{"Recently Added Items"}</h2>
                            { for state.latest_items.iter().map(render_item) }
                        </div>
                        <div class="pure-u-1-3">
                            <h2 class="text-center">{"My Items"}</h2>
                            { for state.user_items.iter().map(render_item) }
                        </div>
                    </div>
                </div>
                <div class="pure-u-1-24"></div>
            </div>
        </div>
    }
}

#[function_component(Home)]
fn home() -> Html {
    let navigator = use_navigator().unwrap();
    let button_disabled = use_state(|| true);
    let item_input = use_node_ref();

    let oninput = {
        let disabled = button_disabled.clone();
        Callback::from(move |e: InputEvent| {
            disabled.set(
                e.target()
                    .unwrap()
                    .unchecked_into::<HtmlInputElement>()
                    .value()
                    .is_empty(),
            )
        })
    };
    let onsubmit = {
        let input = item_input.clone();
        let button_disabled = button_disabled.clone();
        Callback::from(move |e: SubmitEvent| {
            e.prevent_default();
            let item = input.cast::<HtmlInputElement>().unwrap();
            let title = item.value();
            item.set_value("");
            button_disabled.set(true);

            let navigator = navigator.clone();
            spawn_local(async move {
                let response: CreatePollResp = Request::post("/api/poll")
                    .header("Content-Type", "application/json")
                    .body(serde_json::to_string(&CreatePollReq { title }).unwrap())
                    .unwrap()
                    .send()
                    .await
                    .unwrap()
                    .json()
                    .await
                    .unwrap();
                navigator.push(&Route::Poll { id: response.id });
            })
        })
    };

    html! {
        <div class="pure-g">
            <div class="pure-u-1 text-center">
                <h1>{"Anket"}</h1>
                <form class="pure-form" {onsubmit}>
                    <fieldset class="pure-group">
                        <input
                            { oninput }
                            ref={ item_input }
                            type="text"
                            class="pure-input-3-4 margin-auto"
                            placeholder="Poll title"
                        />
                    </fieldset>
                    <fieldset class="pure-group">
                        <button
                            disabled={ *button_disabled }
                            type="submit"
                            class="pure-button pure-input-1-4 pure-button-primary margin-auto"
                        >
                            {"Create Poll"}
                        </button>
                    </fieldset>
                </form>
            </div>
        </div>
    }
}

#[derive(Clone, Routable, PartialEq)]
enum Route {
    #[at("/p/:id")]
    Poll { id: String },
    #[at("/")]
    Home,
    #[not_found]
    #[at("/404")]
    NotFound,
}

fn switch(routes: Route) -> Html {
    match routes {
        Route::Poll { id } => html! { <Poll id={id} /> },
        Route::Home => html! { <Home /> },
        Route::NotFound => html! { <h1>{ "404" }</h1> },
    }
}

#[function_component(App)]
fn app() -> Html {
    html! {
        <BrowserRouter>
            <Switch<Route> render={switch} />
        </BrowserRouter>
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}

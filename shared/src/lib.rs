use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CreatePollReq {
    pub title: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CreatePollResp {
    pub id: String,
    pub title: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Message {
    AddItem { text: String },
    VoteItem { item_id: usize, vote: isize },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ItemState {
    pub id: usize,
    pub text: String,
    pub score: isize,
    pub user_vote: isize,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PollState {
    pub poll_title: String,
    pub top_items: Vec<ItemState>,
    pub latest_items: Vec<ItemState>,
    pub user_items: Vec<ItemState>,
}

use anket_shared::PollState;

use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

/// These oneshot senders are the way that we send response for those operations
#[derive(Debug)]
pub enum PollOp {
    CreatePoll {
        user_id: Uuid,
        title: String,
        // poll id
        response: oneshot::Sender<String>,
    },
    DeletePoll {
        poll_id: String,
        // indicates delete operation success or not
        // this is optional because you may not want to see result of operation
        response: Option<oneshot::Sender<bool>>,
    },
    JoinPoll {
        user_id: Uuid,
        poll_id: String,
        // sender that we pass current state of poll to user
        user_sender: mpsc::UnboundedSender<PollState>,
        // we send poll_sender to user via oneshot response channel
        // we actually send Option<mpsc::Sender> here, because there may not be such a poll
        response: oneshot::Sender<Option<mpsc::Sender<InPollOp>>>,
    },
}

#[derive(Debug)]
pub enum InPollOp {
    Join {
        user_id: Uuid,
        user_sender: mpsc::UnboundedSender<PollState>,
    },
    AddItem {
        user_id: Uuid,
        text: String,
    },
    VoteItem {
        user_id: Uuid,
        item_id: usize,
        value: isize,
    },
}

pub trait ToInPollOp {
    fn to_poll(self, user_id: Uuid) -> InPollOp;
}

impl ToInPollOp for anket_shared::Message {
    fn to_poll(self, user_id: Uuid) -> InPollOp {
        match self {
            anket_shared::Message::AddItem { text } => InPollOp::AddItem { user_id, text },
            anket_shared::Message::VoteItem { item_id, vote } => InPollOp::VoteItem {
                user_id,
                item_id,
                value: vote,
            },
        }
    }
}

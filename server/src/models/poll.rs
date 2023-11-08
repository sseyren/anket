use crate::models::message::*;
use crate::utils::{HashMapVecInsert, RingBuffer, StringKeyGenerate, TouchTimed};

use anket_shared::{ItemState, PollState};

use async_trait::async_trait;
use std::collections::{BTreeSet, HashMap};
use std::ops::RangeInclusive;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::debug;
use uuid::Uuid;

#[async_trait]
pub trait PollOpSender {
    async fn create_poll(&self, user_id: Uuid, title: String) -> String;
    async fn join_poll(
        &self,
        user_id: Uuid,
        poll_id: String,
        user_sender: mpsc::UnboundedSender<PollState>,
    ) -> Option<mpsc::Sender<InPollOp>>;
}

#[async_trait]
impl PollOpSender for mpsc::Sender<PollOp> {
    async fn create_poll(&self, user_id: Uuid, title: String) -> String {
        let (tx, rx) = oneshot::channel();
        self.send(PollOp::CreatePoll {
            user_id,
            title,
            response: tx,
        })
        .await
        .expect("main polls channel should be alive");
        rx.await.expect("oneshot response channel should be alive")
    }
    async fn join_poll(
        &self,
        user_id: Uuid,
        poll_id: String,
        user_sender: mpsc::UnboundedSender<PollState>,
    ) -> Option<mpsc::Sender<InPollOp>> {
        let (tx, rx) = oneshot::channel();
        self.send(PollOp::JoinPoll {
            user_id,
            poll_id,
            user_sender,
            response: tx,
        })
        .await
        .expect("main polls channel should be alive");
        rx.await.expect("oneshot response channel should be alive")
    }
}

pub struct Polls {
    // HashMap<poll id, (sender to communicate with poll worker, poll worker join handle to abort task)>
    polls: HashMap<String, (mpsc::Sender<InPollOp>, JoinHandle<()>)>,
    ops: mpsc::Sender<PollOp>,
}

impl Polls {
    pub fn init() -> (mpsc::Sender<PollOp>, JoinHandle<()>) {
        let (op_sender, mut op_receiver) = mpsc::channel(10_000);
        let mut polls = Self {
            polls: HashMap::new(),
            ops: op_sender.clone(),
        };
        let polls_worker = tokio::spawn(async move {
            debug!("main polls task started");
            while let Some(op) = op_receiver.recv().await {
                polls.process(op).await;
            }
            debug!("main polls task finished");
        });
        (op_sender, polls_worker)
    }
    async fn process(&mut self, op: PollOp) {
        match op {
            PollOp::CreatePoll {
                user_id,
                title,
                response,
            } => {
                let poll_id = self.new_poll(user_id, title, self.ops.clone());
                response
                    .send(poll_id)
                    .unwrap_or_else(|_| debug!("response could not be sent"));
            }
            PollOp::DeletePoll { poll_id, response } => {
                let op = self.delete_poll(&poll_id);
                if let Some(response) = response {
                    response
                        .send(op)
                        .unwrap_or_else(|_| debug!("response could not be sent"));
                }
            }
            PollOp::JoinPoll {
                user_id,
                poll_id,
                user_sender,
                response,
            } => {
                let resp_obj = match self.get_poll(&poll_id) {
                    Some((poll_ops, _)) => {
                        let poll_sender = poll_ops.clone();
                        match poll_sender
                            .send(InPollOp::Join {
                                user_id,
                                user_sender,
                            })
                            .await
                        {
                            Ok(_) => Some(poll_sender),
                            Err(_) => {
                                // poll channel is dropped, delete poll from polls HashMap
                                self.delete_poll(&poll_id);
                                None
                            }
                        }
                    }
                    None => None,
                };
                response
                    .send(resp_obj)
                    .unwrap_or_else(|_| debug!("response could not be sent"));
            }
        }
    }
    fn new_poll(
        &mut self,
        user_id: Uuid,
        title: String,
        polls_ops: mpsc::Sender<PollOp>,
    ) -> String {
        let id = self.polls.generate_key(8);
        let (sender, mut receiver) = mpsc::channel(10_000);

        let poll_id = id.clone();
        let worker_task = tokio::spawn(async move {
            let mut poll = Poll::new(poll_id.clone(), user_id, title);
            let mut timer = tokio::time::interval(Duration::from_millis(500));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            debug!("{} worker started", poll.id);
            loop {
                tokio::select! {
                    _ = timer.tick() => {
                        if *poll.changed.value() {
                            debug!("{} poll.changed, broadcasting...", poll.id);
                            poll.broadcast();
                        } else if poll.changed.elapsed() > Duration::from_secs(15 * 60) {
                            debug!("{} is inactive, worker stops", poll.id);
                            break;
                        }
                    }
                    result = receiver.recv() => {
                        let op = match result {
                            Some(op) => op,
                            None => {
                                debug!("{} receiver is closed, worker stops", poll.id);
                                break;
                            }
                        };
                        debug!("{} received {:?}", poll.id, op);
                        poll.process(op);
                    }
                }
            }
            debug!("{} worker completed, deleting poll...", poll.id);
            polls_ops
                .send(PollOp::DeletePoll {
                    poll_id,
                    response: None,
                })
                .await
                .expect("main polls channel should be alive");
        });

        self.polls.insert(id.clone(), (sender, worker_task));
        id
    }
    fn get_poll(&self, poll_id: &String) -> Option<&(mpsc::Sender<InPollOp>, JoinHandle<()>)> {
        self.polls.get(poll_id)
    }
    fn delete_poll(&mut self, poll_id: &String) -> bool {
        self.polls.remove(poll_id).is_some()
    }
}

struct Poll {
    id: String,
    #[allow(dead_code)]
    user_id: Uuid,
    title: String,

    // indicates that; some changes made and should be calculated & published on the next timer.tick
    changed: TouchTimed<bool>,
    // valid value range for a user item vote
    value_range: RangeInclusive<isize>,

    // item id, item
    items: HashMap<usize, Item>,
    // BTreeSet<(score of item, id of item)>, sorted by scores
    items_by_score: BTreeSet<(isize, usize)>,
    // HashMap<user id, item id>
    items_by_user: HashMap<Uuid, Vec<usize>>,
    // id of item
    last_items: RingBuffer<usize>,

    // connection number, channel sender for send events to user
    audience: HashMap<Uuid, Vec<mpsc::UnboundedSender<PollState>>>,
}

impl Poll {
    fn new(id: String, user_id: Uuid, title: String) -> Poll {
        Poll {
            id,
            user_id,
            title,
            changed: TouchTimed::new(false),
            value_range: -1..=1,
            items: HashMap::new(),
            items_by_score: BTreeSet::new(),
            items_by_user: HashMap::new(),
            last_items: RingBuffer::new(10),
            audience: HashMap::new(),
        }
    }
    fn add_viewer(&mut self, user_id: Uuid, user_sender: mpsc::UnboundedSender<PollState>) {
        self.audience.insert_vec(user_id, user_sender);
    }
    fn add_item(&mut self, user_id: Uuid, item_text: String) -> usize {
        let item_id = self.items.len();
        let item = Item {
            id: item_id,
            user_id: user_id,
            text: item_text,
            score: 0,
            votes: HashMap::new(),
        };

        self.items.insert(item_id, item);
        self.items_by_score.insert((0, item_id));
        self.items_by_user.insert_vec(user_id, item_id);
        self.last_items.push(item_id);

        self.changed.update(true);
        item_id
    }
    fn vote_item(&mut self, user_id: Uuid, item_id: usize, value: isize) {
        if !self.value_range.contains(&value) {
            return;
        }
        if let Some(item) = self.items.get_mut(&item_id) {
            let old_score = item.score;

            // `.insert()` method, updates current vote of this user as well.
            // so, no need to remove existing <user id, value> entry from `item.votes`
            match item.votes.insert(user_id, value) {
                Some(old_value) => item.score += value - old_value,
                None => item.score += value,
            }
            if old_score != item.score {
                if !self.items_by_score.remove(&(old_score, item_id)) {
                    panic!("vote tuple expected in by_score map");
                }
                self.items_by_score.insert((item.score, item_id));

                self.changed.update(true);
            }
        }
    }
    fn get_state(&self, user_id: &Uuid) -> PollState {
        PollState {
            poll_title: self.title.clone(),
            top_items: self
                .items_by_score
                .iter()
                .rev()
                .take(10)
                .map(|(_, item_id)| self.items.get(item_id).unwrap().to_state(user_id))
                .collect(),
            latest_items: self
                .last_items
                .iter()
                .map(|item_id| self.items.get(item_id).unwrap().to_state(user_id))
                .collect(),
            user_items: self
                .items_by_user
                .get(&user_id)
                .unwrap_or(&vec![])
                .iter()
                .rev()
                .map(|item_id| self.items.get(item_id).unwrap().to_state(user_id))
                .collect(),
        }
    }
    fn broadcast(&mut self) {
        let all_users: Vec<Uuid> = self.audience.keys().map(|u| *u).collect();
        for user_id in all_users.iter() {
            let state = self.get_state(user_id);
            let user_senders = self.audience.get_mut(user_id).expect("this should exists");
            user_senders.retain(|sender| match sender.send(state.clone()) {
                Ok(_) => true,
                Err(_) => false,
            });
        }
        self.changed.update(false);
    }
    fn process(&mut self, op: InPollOp) {
        match op {
            InPollOp::Join {
                user_id,
                user_sender,
            } => {
                if !*self.changed.value() {
                    let _ = user_sender.send(self.get_state(&user_id));
                }
                self.add_viewer(user_id, user_sender);
            }
            InPollOp::AddItem { user_id, text } => {
                if !text.is_empty() {
                    let item_id = self.add_item(user_id, text);
                    self.vote_item(user_id, item_id, 1);
                }
            }
            InPollOp::VoteItem {
                user_id,
                item_id,
                value,
            } => {
                self.vote_item(user_id, item_id, value);
            }
        }
    }
}

#[derive(Debug)]
struct Item {
    id: usize, // item id

    #[allow(dead_code)]
    user_id: Uuid, // author id

    text: String,                // text of item
    score: isize,                // computed total score of item
    votes: HashMap<Uuid, isize>, // user id, user vote value
}

impl Item {
    fn to_state(&self, user_id: &Uuid) -> ItemState {
        ItemState {
            id: self.id,
            text: self.text.clone(),
            score: self.score,
            user_vote: *self.votes.get(user_id).unwrap_or(&0),
        }
    }
}

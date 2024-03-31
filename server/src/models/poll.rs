use crate::utils::{HashMapVecInsert, RingBuffer, StringKeyGenerate, TouchTimed};

use anket_shared::{ItemState, PollState};

// TODO remove async_trait dep
use async_trait::async_trait;
use std::collections::{BTreeSet, HashMap};
use std::ops::RangeInclusive;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::debug;
use uuid::Uuid;

pub struct Polls {
    // HashMap<poll id, poll>
    polls: HashMap<String, Arc<Mutex<Poll>>>,
}

impl Polls {
    pub fn new() -> Self {
        Self {
            polls: HashMap::new(),
        }
    }
    pub fn add_poll(&mut self, user_id: Uuid, title: String) -> (String, Arc<Mutex<Poll>>) {
        let id = self.polls.generate_key(8);
        let poll = Poll::new(id.clone(), user_id, title);
        self.polls.insert(id.clone(), poll.clone());
        (id, poll)
    }
    pub fn get_poll(&self, poll_id: &str) -> Option<Arc<Mutex<Poll>>> {
        self.polls.get(poll_id).cloned()
    }
}

pub struct Poll {
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
    // this is an Option, because task created after this
    task: Option<JoinHandle<()>>,
    // TODO bu poll un artik kapanmasi gerektigini, poll HashMap'ine ileten bir channel gerek
}

async fn poll_worker(poll_mutex: Arc<Mutex<Poll>>) {
    let mut timer = tokio::time::interval(Duration::from_millis(500));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    debug!("poll worker started");
    loop {
        timer.tick().await;
        let mut poll = poll_mutex.lock().unwrap();

        if *poll.changed.value() {
            debug!("{} poll.changed, broadcasting...", poll.id);
            poll.broadcast();
        } else if poll.changed.elapsed() > Duration::from_secs(15 * 60) {
            debug!("{} is inactive, worker stops", poll.id);
            break;
        }
    }
    // TODO bu poll un kapatilmasi gerektigini channel'dan bildir
}

impl Poll {
    fn new(id: String, user_id: Uuid, title: String) -> Arc<Mutex<Self>> {
        let mut poll_raw = Self {
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
            task: None,
        };
        let poll = Arc::new(Mutex::new(poll_raw));

        let task = tokio::spawn(poll_worker(poll.clone()));
        poll.lock().unwrap().task = Some(task);

        poll
    }

    pub fn add_viewer(&mut self, user_id: Uuid, user_sender: mpsc::UnboundedSender<PollState>) {
        if !*self.changed.value() {
            // TODO do better error handling; if we can't send this msg to callsite
            // then we shouldn't add this user to audience
            let _ = user_sender.send(self.get_state(&user_id));
        }
        self.audience.insert_vec(user_id, user_sender);
    }

    pub fn add_item(&mut self, user_id: Uuid, item_text: String) -> usize {
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

        // TODO this vote_item call makes some redundant jobs
        self.vote_item(user_id, item_id, 1);
        self.changed.update(true);
        item_id
    }

    pub fn vote_item(&mut self, user_id: Uuid, item_id: usize, value: isize) {
        if !self.value_range.contains(&value) {
            // TODO we can create an error type for this
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
        } else {
            // TODO return error
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

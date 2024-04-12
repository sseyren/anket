use crate::utils::{HashMapVecInsert, RingBuffer, StringKeyGenerate, TouchTimed, UuidKeyGenerate};

use std::collections::{BTreeSet, HashMap};
use std::net::IpAddr;
use std::ops::RangeInclusive;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
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
    pub fn add_poll(
        &mut self,
        settings: PollSettings,
        user_details: UserDetails,
    ) -> (Uuid, Arc<Mutex<Poll>>) {
        let id = self.polls.generate_key(8);
        let (poll, user_id) = Poll::new(id.clone(), settings, user_details);
        self.polls.insert(id, poll.clone());
        (user_id, poll)
    }
    pub fn get_poll(&self, poll_id: &str) -> Option<Arc<Mutex<Poll>>> {
        self.polls.get(poll_id).cloned()
    }
}

#[derive(Clone, Debug)]
pub struct UserDetails {
    pub ip: IpAddr,
    pub id: Option<Uuid>,
    // when we need to get usernames also:
    // pub name: Option<String>,
}

trait UserCollection: Send + Sync {
    fn search_user(&self, details: &UserDetails) -> Option<Uuid>;

    fn get_map(&self) -> &HashMap<Uuid, PollUser>;
    fn get_map_mut(&mut self) -> &mut HashMap<Uuid, PollUser>; // TODO do we need this?

    fn create_user(&mut self, details: UserDetails) -> Result<Uuid, UserCreateError>;
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum UserLookupMethod {
    IPBased,
    SessionBased,
}
impl Into<Box<dyn UserCollection>> for UserLookupMethod {
    fn into(self) -> Box<dyn UserCollection> {
        match self {
            UserLookupMethod::IPBased => Box::new(IPBasedUsers::new()) as Box<dyn UserCollection>,
            UserLookupMethod::SessionBased => {
                Box::new(PlainUsers::new()) as Box<dyn UserCollection>
            }
        }
    }
}

struct PlainUsers {
    users: HashMap<Uuid, PollUser>,
}
impl PlainUsers {
    fn new() -> Self {
        Self {
            users: HashMap::new(),
        }
    }
}
impl UserCollection for PlainUsers {
    fn search_user(&self, details: &UserDetails) -> Option<Uuid> {
        match details.id {
            Some(id) => match self.users.get(&id) {
                Some(user) => Some(user.id),
                None => None,
            },
            None => None,
        }
    }

    fn get_map(&self) -> &HashMap<Uuid, PollUser> {
        &self.users
    }
    fn get_map_mut(&mut self) -> &mut HashMap<Uuid, PollUser> {
        &mut self.users
    }

    fn create_user(&mut self, details: UserDetails) -> Result<Uuid, UserCreateError> {
        let id = self.users.generate_key();
        self.users.insert(id, PollUser::new(id));
        Ok(id)
    }
}

struct IPBasedUsers {
    users: HashMap<Uuid, PollUser>,
    users_by_ip: HashMap<IpAddr, Uuid>,
}
impl IPBasedUsers {
    fn new() -> Self {
        Self {
            users: HashMap::new(),
            users_by_ip: HashMap::new(),
        }
    }
}
impl UserCollection for IPBasedUsers {
    fn search_user(&self, details: &UserDetails) -> Option<Uuid> {
        self.users_by_ip.get(&details.ip).cloned()
    }

    fn get_map(&self) -> &HashMap<Uuid, PollUser> {
        &self.users
    }
    fn get_map_mut(&mut self) -> &mut HashMap<Uuid, PollUser> {
        &mut self.users
    }

    fn create_user(&mut self, details: UserDetails) -> Result<Uuid, UserCreateError> {
        if self.users_by_ip.get(&details.ip).is_some() {
            return Err(UserCreateError::UserAlreadyExists);
        }
        let id = self.users.generate_key();
        self.users.insert(id, PollUser::new(id));
        self.users_by_ip.insert(details.ip, id);
        Ok(id)
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PollSettings {
    pub title: String,
    pub user_lookup_method: UserLookupMethod,
}

struct PollUser {
    id: Uuid,
    // user may have opened multiple browser tabs to same poll
    // this is because we have a vec here, insted of single sender
    senders: Vec<mpsc::UnboundedSender<crate::views::UserResponse>>,
    // we may add UserDetails here to make easy to delete users from `UserLookup` implementations
}
impl PollUser {
    fn new(id: Uuid) -> Self {
        Self {
            id,
            senders: Vec::with_capacity(1),
        }
    }
}

pub struct Poll {
    id: String,
    title: String,
    owner: Uuid, // user id

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

    users: Box<dyn UserCollection>,

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
    fn new(
        id: String,
        settings: PollSettings,
        user_details: UserDetails,
    ) -> (Arc<Mutex<Self>>, Uuid) {
        let mut users: Box<dyn UserCollection> = settings.user_lookup_method.into();
        let owner_id = users
            .create_user(user_details)
            .expect("this is the first user that we create on this poll");

        let poll_raw = Self {
            id,
            owner: owner_id,
            title: settings.title,
            changed: TouchTimed::new(false),
            value_range: -1..=1,
            items: HashMap::new(),
            items_by_score: BTreeSet::new(),
            items_by_user: HashMap::new(),
            last_items: RingBuffer::new(10),
            users,
            task: None,
        };
        let poll = Arc::new(Mutex::new(poll_raw));

        let task = tokio::spawn(poll_worker(poll.clone()));
        poll.lock().unwrap().task = Some(task);

        (poll, owner_id)
    }

    pub fn get_id(&self) -> &str {
        &self.id
    }

    pub fn join(
        &mut self,
        user_details: UserDetails,
        user_sender: mpsc::UnboundedSender<crate::views::UserResponse>,
    ) -> Uuid {
        let user_id = if let Some(user_id) = self.users.search_user(&user_details) {
            user_id
        } else {
            self.users
                .create_user(user_details)
                .expect("this user does not exists in poll")
        };

        if !*self.changed.value() {
            // no need to examine error here, because sender is going to be
            // dropped on next broadcast if it's erroneous
            let _ = user_sender.send(self.get_state(&user_id));
        }
        self.users
            .get_map_mut()
            .get_mut(&user_id)
            .expect("we just got/created this user")
            .senders
            .push(user_sender);

        // TODO return a UserDetails instead
        user_id
    }

    // we don't need to check validity of `user_id` on add_item() & vote_item()
    // because, in order to use these method, they need to call join first

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

    pub fn vote_item(
        &mut self,
        user_id: Uuid,
        item_id: usize,
        value: isize,
    ) -> Result<(), VotePollItemError> {
        if !self.value_range.contains(&value) {
            return Err(VotePollItemError::InvalidValue);
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
            return Err(VotePollItemError::ItemNotFound);
        }
        Ok(())
    }

    fn get_state(&self, user_id: &Uuid) -> crate::views::UserResponse {
        crate::views::UserResponse::PollStateUpdate(PollState {
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
        })
    }

    fn broadcast(&mut self) {
        let all_users: Vec<Uuid> = self.users.get_map().keys().copied().collect();
        for user_id in all_users.iter() {
            let state = self.get_state(user_id);
            self.users
                .get_map_mut()
                .get_mut(user_id)
                .expect("user exists because we iterate same map")
                .senders
                .retain(|sender| sender.send(state.clone()).is_ok());
        }
        /*
        for (user_id, user) in self.users.get_map_mut().iter_mut() {
            let state = self.get_state(user_id);
            user.senders
                .retain(|sender| sender.send(state.clone()).is_ok());
        }
        */
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

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ItemState {
    pub id: usize,
    pub text: String,
    pub score: isize,
    pub user_vote: isize,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PollState {
    pub poll_title: String,
    pub top_items: Vec<ItemState>,
    pub latest_items: Vec<ItemState>,
    pub user_items: Vec<ItemState>,
}

#[derive(Debug, Error)]
pub enum UserCreateError {
    #[error("You can't add this user to poll, this user already exists.")]
    UserAlreadyExists,
    // TODO add not enough details provided error
}

#[derive(Debug, Error)]
pub enum VotePollItemError {
    // TODO add more info fields to this enum branch
    #[error("Provided vote value is invalid for this poll item.")]
    InvalidValue,
    #[error("No such item exists with this item ID.")]
    ItemNotFound,
}

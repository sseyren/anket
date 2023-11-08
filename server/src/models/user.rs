use crate::utils::{StringKeyGenerate, UuidKeyGenerate};

use std::collections::HashMap;
use std::net::IpAddr;
use uuid::Uuid;

pub struct Users {
    users: HashMap<Uuid, User>,
    users_by_session: HashMap<String, Uuid>,
    users_by_ip: HashMap<IpAddr, Uuid>,
}

impl Users {
    pub fn init() -> Self {
        Self {
            users: HashMap::new(),
            users_by_session: HashMap::new(),
            users_by_ip: HashMap::new(),
        }
    }
    pub fn new_user(&mut self, ip: IpAddr) -> &User {
        let id = self.users.generate_key();
        let session = self.users_by_session.generate_key(96);

        self.users_by_session.insert(session.clone(), id);
        self.users_by_ip.insert(ip, id);
        // This will always insert because we select a unique key by `.generate_key()` above
        &*self.users.entry(id).or_insert(User { id, ip, session })
    }
    pub fn get_user(&self, user_id: &Uuid) -> Option<&User> {
        self.users.get(user_id)
    }
    pub fn get_user_by_session(&self, session: &str) -> Option<&User> {
        match self.users_by_session.get(session) {
            Some(uuid) => Some(self.get_user(uuid).expect(
                "user found on session map but not found on owner map, this is unexpected",
            )),
            None => None,
        }
    }
    pub fn get_user_by_ip(&self, ip: &IpAddr) -> Option<&User> {
        match self.users_by_ip.get(ip) {
            Some(uuid) => Some(self.get_user(uuid).expect(
                "user found on session map but not found on owner map, this is unexpected",
            )),
            None => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct User {
    pub id: Uuid,
    pub ip: IpAddr,
    pub session: String,
}

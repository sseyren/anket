// TODO remove this completely

use crate::utils::StringKeyGenerate;

use std::collections::HashSet;
use std::ops::{Deref, DerefMut};

const SESSION_KEY_LENGTH: usize = 96;

pub fn is_session_valid(session: &str) -> bool {
    !session.is_empty()
}

pub struct SessionSet(HashSet<String>);

impl Deref for SessionSet {
    type Target = HashSet<String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for SessionSet {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl SessionSet {
    pub fn new() -> Self {
        Self(HashSet::new())
    }
    pub fn new_session(&mut self) -> String {
        self.generate_key(SESSION_KEY_LENGTH)
    }
}

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::str::FromStr;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub trait StringKeyGenerate {
    fn generate_key(&self, length: usize) -> String;
}
impl<V> StringKeyGenerate for HashMap<String, V> {
    fn generate_key(&self, length: usize) -> String {
        use rand::distributions::{Alphanumeric, DistString};
        let mut key: String;
        loop {
            key = Alphanumeric.sample_string(&mut rand::thread_rng(), length);
            if !self.contains_key(&key) {
                break;
            }
        }
        key
    }
}

pub trait UuidKeyGenerate {
    fn generate_key(&self) -> Uuid;
}
impl<V> UuidKeyGenerate for HashMap<Uuid, V> {
    fn generate_key(&self) -> Uuid {
        let mut key: Uuid;
        loop {
            key = Uuid::new_v4();
            if !self.contains_key(&key) {
                break;
            }
        }
        key
    }
}

pub trait HashMapVecInsert<K, V>
where
    K: Eq + PartialEq + std::hash::Hash,
{
    fn insert_vec(&mut self, key: K, value: V);
}
impl<K, V> HashMapVecInsert<K, V> for HashMap<K, Vec<V>>
where
    K: Eq + PartialEq + std::hash::Hash,
{
    fn insert_vec(&mut self, key: K, value: V) {
        match self.get_mut(&key) {
            Some(vec) => {
                vec.push(value);
            }
            None => {
                self.insert(key, vec![value]);
            }
        }
    }
}

#[derive(Debug)]
pub struct RingBuffer<T> {
    vec: VecDeque<T>,
    capacity: usize,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            vec: VecDeque::with_capacity(capacity),
            capacity,
        }
    }
    pub fn push(&mut self, item: T) {
        if self.vec.len() == self.capacity {
            self.vec.pop_back();
        }
        self.vec.push_front(item);
    }
    #[allow(dead_code)]
    pub fn pop(&mut self) -> Option<T> {
        self.vec.pop_back()
    }
    pub fn iter(&self) -> std::collections::vec_deque::Iter<'_, T> {
        self.vec.iter()
    }
}

pub struct TouchTimed<T> {
    value: T,
    last_update: Instant,
}

impl<T> TouchTimed<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            last_update: Instant::now(),
        }
    }
    pub fn value(&self) -> &T {
        &self.value
    }
    pub fn update(&mut self, value: T) {
        self.value = value;
        self.last_update = Instant::now();
    }
    pub fn elapsed(&self) -> Duration {
        self.last_update.elapsed()
    }
}

/// Extracts left-most IP address from given `X-Forwarded-For` HTTP header.
pub fn forwarded_header_ip(header_value: &axum::http::header::HeaderValue) -> Option<IpAddr> {
    IpAddr::from_str(
        std::str::from_utf8(header_value.as_bytes())
            .ok()?
            .split_once(',')?
            .0
            .trim(),
    )
    .ok()
}

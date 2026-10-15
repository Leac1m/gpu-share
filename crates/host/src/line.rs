//! The line of waiting requests (spec §6.8): one answered at a time, first come first served,
//! at most one request per paired device (`P2-T6`, `P2-T10`). Pure bookkeeping, no async; the
//! gatekeeper wraps it in [`crate::queue`].

use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinError {
    /// This device already has a request waiting or being answered.
    AlreadyQueued,
}

#[derive(Debug)]
struct Entry<T> {
    id: u64,
    device: String,
    payload: T,
}

#[derive(Debug)]
pub struct Line<T> {
    waiting: VecDeque<Entry<T>>,
    running: Option<Entry<T>>,
    next_id: u64,
}

impl<T> Default for Line<T> {
    fn default() -> Self {
        Self { waiting: VecDeque::new(), running: None, next_id: 1 }
    }
}

impl<T> Line<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Joins the back of the line.
    pub fn join(&mut self, device: &str, payload: T) -> Result<u64, JoinError> {
        if self.has_device(device) {
            return Err(JoinError::AlreadyQueued);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.waiting.push_back(Entry { id, device: device.to_string(), payload });
        Ok(id)
    }

    /// Joins the front of the waiting line, behind the request being answered (the owner's own
    /// questions, `P9-T1`).
    pub fn join_front(&mut self, device: &str, payload: T) -> Result<u64, JoinError> {
        if self.has_device(device) {
            return Err(JoinError::AlreadyQueued);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.waiting.push_front(Entry { id, device: device.to_string(), payload });
        Ok(id)
    }

    /// The request that would start next.
    pub fn front(&self) -> Option<&T> {
        self.waiting.front().map(|e| &e.payload)
    }

    pub fn has_device(&self, device: &str) -> bool {
        self.running.iter().chain(self.waiting.iter()).any(|e| e.device == device)
    }

    /// Place in line as the client is told it: 1 = next (nothing ahead), 2 = one request ahead,
    /// counting the one being answered. `None` once the request is running or gone.
    pub fn position(&self, id: u64) -> Option<u32> {
        let i = self.waiting.iter().position(|e| e.id == id)?;
        Some(i as u32 + 1 + u32::from(self.running.is_some()))
    }

    /// Moves the front of the line to running, if nothing is running. Returns its id.
    pub fn start_next(&mut self) -> Option<u64> {
        if self.running.is_some() {
            return None;
        }
        self.running = self.waiting.pop_front();
        self.running.as_ref().map(|e| e.id)
    }

    /// Removes a request wherever it is (finished, cancelled, client gone). True if found.
    pub fn leave(&mut self, id: u64) -> bool {
        if self.running.as_ref().is_some_and(|e| e.id == id) {
            self.running = None;
            return true;
        }
        let before = self.waiting.len();
        self.waiting.retain(|e| e.id != id);
        before != self.waiting.len()
    }

    /// Empties the line, running request included (a pause turns everyone away).
    pub fn drain(&mut self) -> Vec<(u64, T)> {
        self.running.take().into_iter().chain(self.waiting.drain(..)).map(|e| (e.id, e.payload)).collect()
    }

    /// Removes every request, running or waiting, whose payload matches (a pause turns friends
    /// away but not the owner).
    pub fn drain_where(&mut self, mut f: impl FnMut(&T) -> bool) -> Vec<(u64, T)> {
        let mut out = Vec::new();
        if self.running.as_ref().is_some_and(|e| f(&e.payload)) {
            let e = self.running.take().unwrap();
            out.push((e.id, e.payload));
        }
        let (gone, kept): (Vec<_>, Vec<_>) = self.waiting.drain(..).partition(|e| f(&e.payload));
        self.waiting = kept.into();
        out.extend(gone.into_iter().map(|e| (e.id, e.payload)));
        out
    }

    /// Removes everything from one device (it was unpaired).
    pub fn remove_device(&mut self, device: &str) -> Vec<(u64, T)> {
        let mut out = Vec::new();
        if self.running.as_ref().is_some_and(|e| e.device == device) {
            let e = self.running.take().unwrap();
            out.push((e.id, e.payload));
        }
        let (gone, kept): (Vec<_>, Vec<_>) = self.waiting.drain(..).partition(|e| e.device == device);
        self.waiting = kept.into();
        out.extend(gone.into_iter().map(|e| (e.id, e.payload)));
        out
    }

    pub fn running(&self) -> Option<(u64, &str, &T)> {
        self.running.as_ref().map(|e| (e.id, e.device.as_str(), &e.payload))
    }

    /// Devices waiting, front first.
    pub fn waiting_devices(&self) -> Vec<String> {
        self.waiting.iter().map(|e| e.device.clone()).collect()
    }

    pub fn iter_waiting(&self) -> impl Iterator<Item = (u64, &T)> {
        self.waiting.iter().map(|e| (e.id, &e.payload))
    }

    pub fn payload(&self, id: u64) -> Option<&T> {
        self.running.iter().chain(self.waiting.iter()).find(|e| e.id == id).map(|e| &e.payload)
    }

    pub fn payload_mut(&mut self, id: u64) -> Option<&mut T> {
        self.running.iter_mut().chain(self.waiting.iter_mut()).find(|e| e.id == id).map(|e| &mut e.payload)
    }

    pub fn is_empty(&self) -> bool {
        self.running.is_none() && self.waiting.is_empty()
    }
}

#[cfg(test)]
mod owner_tests {
    use super::*;

    #[test]
    fn the_front_of_the_line_is_behind_the_running_request() {
        let mut l: Line<&str> = Line::new();
        let a = l.join("a", "friend").unwrap();
        assert_eq!(l.start_next(), Some(a));
        let b = l.join("b", "friend").unwrap();
        let o = l.join_front("owner", "owner").unwrap();
        assert_eq!(l.position(o), Some(2));
        assert_eq!(l.position(b), Some(3));
        assert_eq!(l.join_front("owner", "owner"), Err(JoinError::AlreadyQueued));
        assert_eq!(l.front(), Some(&"owner"));
    }

    #[test]
    fn drain_where_keeps_what_does_not_match() {
        let mut l: Line<&str> = Line::new();
        let o = l.join("owner", "owner").unwrap();
        l.start_next();
        let f = l.join("f", "friend").unwrap();
        let gone = l.drain_where(|p| *p != "owner");
        assert_eq!(gone.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![f]);
        assert_eq!(l.running().map(|r| r.0), Some(o));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_come_first_served_one_at_a_time() {
        let mut l = Line::new();
        let a = l.join("tunde", ()).unwrap();
        let b = l.join("you", ()).unwrap();
        assert_eq!(l.start_next(), Some(a));
        assert_eq!(l.start_next(), None, "only one runs at a time");
        assert_eq!(l.position(b), Some(2), "one request ahead of yours");
        l.leave(a);
        assert_eq!(l.position(b), Some(1));
        assert_eq!(l.start_next(), Some(b));
        assert_eq!(l.position(b), None);
    }

    #[test]
    fn one_request_per_device_counting_the_running_one() {
        let mut l = Line::new();
        let a = l.join("you", ()).unwrap();
        assert_eq!(l.join("you", ()), Err(JoinError::AlreadyQueued));
        l.start_next();
        assert_eq!(l.join("you", ()), Err(JoinError::AlreadyQueued), "still being answered");
        l.leave(a);
        assert!(l.join("you", ()).is_ok());
    }

    #[test]
    fn positions_count_everything_ahead() {
        let mut l = Line::new();
        let ids: Vec<u64> = ["a", "b", "c", "d"].iter().map(|d| l.join(d, ()).unwrap()).collect();
        l.start_next();
        assert_eq!(ids.iter().map(|&i| l.position(i)).collect::<Vec<_>>(), vec![None, Some(2), Some(3), Some(4)]);
        l.leave(ids[2]); // c cancels
        assert_eq!(l.position(ids[3]), Some(3));
    }

    #[test]
    fn cancelling_while_waiting_never_disturbs_the_running_request() {
        let mut l = Line::new();
        let a = l.join("a", ()).unwrap();
        let b = l.join("b", ()).unwrap();
        l.start_next();
        assert!(l.leave(b));
        assert_eq!(l.running().map(|r| r.0), Some(a));
        assert!(!l.leave(b), "leaving twice is harmless");
    }

    #[test]
    fn a_pause_turns_everyone_away() {
        let mut l = Line::new();
        for d in ["a", "b", "c"] {
            l.join(d, d.to_string()).unwrap();
        }
        l.start_next();
        let gone: Vec<String> = l.drain().into_iter().map(|(_, p)| p).collect();
        assert_eq!(gone, vec!["a", "b", "c"], "running first, then the line in order");
        assert!(l.is_empty());
    }

    #[test]
    fn removing_a_device_takes_only_its_requests() {
        let mut l = Line::new();
        l.join("a", 1).unwrap();
        l.join("b", 2).unwrap();
        l.join("c", 3).unwrap();
        l.start_next();
        assert_eq!(l.remove_device("a").len(), 1);
        assert_eq!(l.remove_device("c").len(), 1);
        assert_eq!(l.waiting_devices(), vec!["b"]);
        assert!(l.running().is_none());
    }
}

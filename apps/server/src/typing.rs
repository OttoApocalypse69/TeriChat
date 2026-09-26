//! Ephemeral typing signals.
//!
//! A signal says "this account is typing in this conversation right now". It
//! is relayed live to the other members' gateway sockets and nothing else:
//! never stored, never replayed, never part of an event id or resume cursor.
//! It carries no content, but it is server-visible activity metadata; clients
//! expire it on their own after a few seconds.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::broadcast;
use uuid::Uuid;

/// Live buffer. Typing is best effort: a lagging socket simply skips signals.
const CAPACITY: usize = 256;
/// One signal per account and conversation per interval; clients re-send
/// while typing, so a busy typist still shows as typing on every screen.
pub const MIN_INTERVAL: Duration = Duration::from_secs(2);
/// Throttle entries kept before stale ones are swept (bounded memory).
const THROTTLE_ENTRIES: usize = 4096;

/// One typing signal, addressed to an explicit recipient set resolved when
/// it was published (the conversation's participants at that moment).
#[derive(Debug, Clone)]
pub struct TypingSignal {
    /// Conversation being typed in.
    pub conversation_id: Uuid,
    /// Account that is typing.
    pub user_id: Uuid,
    /// Accounts the signal was addressed to when published. A prefilter
    /// only: the gateway re-checks each recipient's access at delivery.
    pub recipients: Arc<[Uuid]>,
    /// Highest message seq in the conversation when the signal was
    /// published. Clients drop a signal older than a message the typist has
    /// since sent, since the two travel on independent streams.
    pub last_seq: i64,
}

/// Per-router fan-out for typing signals, with a publish throttle.
#[derive(Clone)]
pub struct TypingBus {
    tx: broadcast::Sender<TypingSignal>,
    last: Arc<Mutex<HashMap<(Uuid, Uuid), Instant>>>,
}

impl Default for TypingBus {
    fn default() -> Self {
        Self::new()
    }
}

impl TypingBus {
    /// A bus with no subscribers yet.
    #[must_use]
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CAPACITY);
        Self {
            tx,
            last: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Receive signals published from now on.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<TypingSignal> {
        self.tx.subscribe()
    }

    /// Reserve this account's publish slot for the conversation, before any
    /// database work, unless it already signalled within [`MIN_INTERVAL`].
    /// A reservation dropped without publishing is released, so a request
    /// that fails authorization does not throttle the next attempt.
    #[must_use]
    pub fn admit(&self, user_id: Uuid, conversation_id: Uuid) -> Option<Admission> {
        let now = Instant::now();
        let key = (user_id, conversation_id);
        let mut last = self.slots();
        if last
            .get(&key)
            .is_some_and(|at| now.duration_since(*at) < MIN_INTERVAL)
        {
            return None;
        }
        if last.len() >= THROTTLE_ENTRIES {
            last.retain(|_, at| now.duration_since(*at) < MIN_INTERVAL);
        }
        last.insert(key, now);
        Some(Admission {
            bus: self.clone(),
            key,
            at: now,
            published: false,
        })
    }

    fn slots(&self) -> std::sync::MutexGuard<'_, HashMap<(Uuid, Uuid), Instant>> {
        // A poisoned lock only means another publisher panicked mid-insert;
        // the map is still a valid throttle, so keep using it.
        self.last
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// An admitted typing request, holding its throttle slot until published.
pub struct Admission {
    bus: TypingBus,
    key: (Uuid, Uuid),
    at: Instant,
    published: bool,
}

impl Admission {
    /// Relay the signal to `recipients`, keeping the slot for the interval.
    pub fn publish(mut self, recipients: Arc<[Uuid]>, last_seq: i64) {
        self.published = true;
        let (user_id, conversation_id) = self.key;
        // No live subscriber is not an error: nobody is watching right now.
        let _ = self.bus.tx.send(TypingSignal {
            conversation_id,
            user_id,
            recipients,
            last_seq,
        });
    }
}

impl Drop for Admission {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        let mut last = self.bus.slots();
        if last.get(&self.key) == Some(&self.at) {
            last.remove(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn publish(bus: &TypingBus, user: Uuid, conversation: Uuid) -> bool {
        bus.admit(user, conversation)
            .map(|admission| admission.publish(Arc::from(vec![user]), 0))
            .is_some()
    }

    #[test]
    fn throttles_per_account_and_conversation() {
        let bus = TypingBus::new();
        let mut rx = bus.subscribe();
        let (ada, bob, room) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        assert!(publish(&bus, ada, room));
        assert!(!publish(&bus, ada, room), "same typist, same room");
        assert!(publish(&bus, bob, room), "another typist");
        assert!(publish(&bus, ada, Uuid::now_v7()), "another room");
        let relayed: Vec<Uuid> = std::iter::from_fn(|| rx.try_recv().ok())
            .map(|s| s.user_id)
            .collect();
        assert_eq!(relayed, vec![ada, bob, ada]);
    }

    #[test]
    fn unpublished_admissions_release_their_slot() {
        let bus = TypingBus::new();
        let mut rx = bus.subscribe();
        let (ada, room) = (Uuid::now_v7(), Uuid::now_v7());
        let pending = bus.admit(ada, room).expect("first request is admitted");
        assert!(
            bus.admit(ada, room).is_none(),
            "a concurrent repeat waits for the first"
        );
        drop(pending);
        assert!(rx.try_recv().is_err(), "a dropped admission relays nothing");
        assert!(publish(&bus, ada, room), "a failed attempt never throttles");
    }

    #[test]
    fn stale_throttle_entries_are_swept_at_capacity() {
        let bus = TypingBus::new();
        let stale = Instant::now()
            .checked_sub(MIN_INTERVAL * 2)
            .expect("monotonic clock has history");
        {
            let mut last = bus.last.lock().unwrap();
            for _ in 0..THROTTLE_ENTRIES {
                last.insert((Uuid::now_v7(), Uuid::now_v7()), stale);
            }
        }
        assert!(publish(&bus, Uuid::now_v7(), Uuid::now_v7()));
        assert_eq!(
            bus.last.lock().unwrap().len(),
            1,
            "only the fresh entry remains"
        );
    }
}

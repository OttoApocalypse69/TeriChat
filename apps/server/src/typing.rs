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
    /// Accounts allowed to see the signal. The typist is excluded at delivery.
    pub recipients: Arc<[Uuid]>,
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

    /// Relay a signal unless the same account signalled this conversation
    /// within [`MIN_INTERVAL`]. Returns whether it was relayed.
    pub fn publish(&self, signal: TypingSignal) -> bool {
        let now = Instant::now();
        let key = (signal.user_id, signal.conversation_id);
        {
            // A poisoned lock only means another publisher panicked mid-insert;
            // the map is still a valid throttle, so keep using it.
            let mut last = self
                .last
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if last
                .get(&key)
                .is_some_and(|at| now.duration_since(*at) < MIN_INTERVAL)
            {
                return false;
            }
            if last.len() >= THROTTLE_ENTRIES {
                last.retain(|_, at| now.duration_since(*at) < MIN_INTERVAL);
            }
            last.insert(key, now);
        }
        // No live subscriber is not an error: nobody is watching right now.
        let _ = self.tx.send(signal);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(user: Uuid, conversation: Uuid) -> TypingSignal {
        TypingSignal {
            conversation_id: conversation,
            user_id: user,
            recipients: Arc::from(vec![user]),
        }
    }

    #[test]
    fn throttles_per_account_and_conversation() {
        let bus = TypingBus::new();
        let mut rx = bus.subscribe();
        let (ada, bob, room) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        assert!(bus.publish(signal(ada, room)));
        assert!(!bus.publish(signal(ada, room)), "same typist, same room");
        assert!(bus.publish(signal(bob, room)), "another typist");
        assert!(bus.publish(signal(ada, Uuid::now_v7())), "another room");
        let relayed: Vec<Uuid> = std::iter::from_fn(|| rx.try_recv().ok())
            .map(|s| s.user_id)
            .collect();
        assert_eq!(relayed, vec![ada, bob, ada]);
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
        assert!(bus.publish(signal(Uuid::now_v7(), Uuid::now_v7())));
        assert_eq!(
            bus.last.lock().unwrap().len(),
            1,
            "only the fresh entry remains"
        );
    }
}

//! In-memory, TTL-bounded store of live content keys — the
//! server-side half of "session-bound decryption" (ARCH.md §3.4).
//!
//! A logged-in, unlocked client `POST /v1/t/:tid/session-key` with
//! the raw content key; the server caches it here keyed by
//! `tenant_id` with a short expiry. While the entry is live, vault
//! reads and writes for that tenant decrypt/encrypt server-side.
//! When no client is online (or the client forgets to heartbeat),
//! the entry lapses and reads of encrypted rows return
//! `vault_locked`.
//!
//! Keys never touch disk. A process restart re-locks every tenant
//! until the next client reconnects.

use chrono::{DateTime, Duration, Utc};
use std::{collections::HashMap, sync::Mutex};
use uuid::Uuid;

/// Default publish TTL. A client heartbeat at a fraction of this
/// (say ~1/4) keeps the entry live; miss the window and the next
/// vault op on that tenant 423s until reconnect.
pub const DEFAULT_TTL: Duration = Duration::minutes(15);

struct Entry {
    key: [u8; 32],
    expires_at: DateTime<Utc>,
    /// Session that published this key. Used for audit + revoke
    /// (logging out a session drops any keys it had published).
    session_id: Uuid,
}

pub struct SessionKeyStore {
    // `std::sync::Mutex` is fine here — ops are O(1) and never await.
    inner: Mutex<HashMap<Uuid, Entry>>,
}

pub struct PublishOutcome {
    pub expires_at: DateTime<Utc>,
}

impl Default for SessionKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionKeyStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Publish or refresh a content key. Subsequent `get`s for
    /// `tenant_id` succeed until `expires_at`.
    pub fn publish(
        &self,
        tenant_id: Uuid,
        session_id: Uuid,
        key: [u8; 32],
        ttl: Duration,
    ) -> PublishOutcome {
        let expires_at = Utc::now() + ttl;
        let mut g = self.inner.lock().unwrap();
        g.insert(
            tenant_id,
            Entry {
                key,
                expires_at,
                session_id,
            },
        );
        PublishOutcome { expires_at }
    }

    /// Fetch the live key bytes for a tenant, if any. Expired entries
    /// self-evict on the read path.
    pub fn get(&self, tenant_id: Uuid) -> Option<[u8; 32]> {
        let mut g = self.inner.lock().unwrap();
        let entry = g.get(&tenant_id)?;
        if entry.expires_at <= Utc::now() {
            g.remove(&tenant_id);
            return None;
        }
        Some(entry.key)
    }

    /// Drop the key for a tenant. Idempotent.
    pub fn revoke(&self, tenant_id: Uuid) {
        let mut g = self.inner.lock().unwrap();
        g.remove(&tenant_id);
    }

    /// Drop any keys published by the given session. Called when a
    /// session logs out so the server doesn't keep decrypting on
    /// behalf of a revoked principal.
    pub fn revoke_for_session(&self, session_id: Uuid) {
        let mut g = self.inner.lock().unwrap();
        g.retain(|_, e| e.session_id != session_id);
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_then_get() {
        let store = SessionKeyStore::new();
        let tid = Uuid::new_v4();
        let sid = Uuid::new_v4();
        store.publish(tid, sid, [7u8; 32], DEFAULT_TTL);
        assert_eq!(store.get(tid), Some([7u8; 32]));
    }

    #[test]
    fn expired_entries_evict() {
        let store = SessionKeyStore::new();
        let tid = Uuid::new_v4();
        let sid = Uuid::new_v4();
        store.publish(tid, sid, [1u8; 32], Duration::seconds(-1));
        assert!(store.get(tid).is_none());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn revoke_drops_entry() {
        let store = SessionKeyStore::new();
        let tid = Uuid::new_v4();
        let sid = Uuid::new_v4();
        store.publish(tid, sid, [2u8; 32], DEFAULT_TTL);
        store.revoke(tid);
        assert!(store.get(tid).is_none());
    }

    #[test]
    fn revoke_for_session_clears_all_the_session_published() {
        let store = SessionKeyStore::new();
        let sid = Uuid::new_v4();
        let other = Uuid::new_v4();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        store.publish(a, sid, [1u8; 32], DEFAULT_TTL);
        store.publish(b, other, [2u8; 32], DEFAULT_TTL);
        store.revoke_for_session(sid);
        assert!(store.get(a).is_none());
        assert!(store.get(b).is_some());
    }
}

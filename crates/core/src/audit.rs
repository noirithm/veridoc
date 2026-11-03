use serde::{Deserialize, Serialize};

use crate::clock::now_ms;
use crate::hash::sha256_hex;
use crate::{Error, Result};

const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One record in the audit log. Each entry commits to the hash of the
/// one before it, so editing or removing history is detectable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub index: u64,
    pub timestamp_ms: u64,
    pub actor: String,
    pub action: String,
    pub detail: String,
    pub prev_hash: String,
    pub hash: String,
}

impl AuditEntry {
    fn compute_hash(
        index: u64,
        timestamp_ms: u64,
        actor: &str,
        action: &str,
        detail: &str,
        prev_hash: &str,
    ) -> String {
        let payload =
            serde_json::to_string(&(index, timestamp_ms, actor, action, detail, prev_hash))
                .expect("primitive tuple always serializes");
        sha256_hex(payload.as_bytes())
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AuditLog {
    entries: Vec<AuditEntry>,
}

impl AuditLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entries(&self) -> &[AuditEntry] {
        &self.entries
    }

    /// Hash of the latest entry, or the genesis hash for an empty log.
    pub fn head_hash(&self) -> &str {
        self.entries
            .last()
            .map(|e| e.hash.as_str())
            .unwrap_or(GENESIS_HASH)
    }

    pub fn append(
        &mut self,
        actor: impl Into<String>,
        action: impl Into<String>,
        detail: impl Into<String>,
    ) -> &AuditEntry {
        let actor = actor.into();
        let action = action.into();
        let detail = detail.into();
        let index = self.entries.len() as u64;
        let timestamp_ms = now_ms();
        let prev_hash = self.head_hash().to_string();
        let hash =
            AuditEntry::compute_hash(index, timestamp_ms, &actor, &action, &detail, &prev_hash);

        self.entries.push(AuditEntry {
            index,
            timestamp_ms,
            actor,
            action,
            detail,
            prev_hash,
            hash,
        });
        self.entries.last().expect("entry was just pushed")
    }

    /// Recomputes every hash and checks the chain links.
    pub fn verify(&self) -> Result<()> {
        let mut expected_prev = GENESIS_HASH;

        for (i, entry) in self.entries.iter().enumerate() {
            if entry.index != i as u64 {
                return Err(Error::BrokenChain {
                    index: entry.index,
                    reason: "index out of sequence",
                });
            }
            if entry.prev_hash != expected_prev {
                return Err(Error::BrokenChain {
                    index: entry.index,
                    reason: "previous hash does not match",
                });
            }
            let recomputed = AuditEntry::compute_hash(
                entry.index,
                entry.timestamp_ms,
                &entry.actor,
                &entry.action,
                &entry.detail,
                &entry.prev_hash,
            );
            if entry.hash != recomputed {
                return Err(Error::BrokenChain {
                    index: entry.index,
                    reason: "entry contents were altered",
                });
            }
            expected_prev = &entry.hash;
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Parses a log and rejects it if the chain does not verify.
    pub fn from_json(json: &str) -> Result<Self> {
        let log: AuditLog = serde_json::from_str(json)?;
        log.verify()?;
        Ok(log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_log() -> AuditLog {
        let mut log = AuditLog::new();
        log.append("system", "document_received", "invoice.pdf");
        log.append("system", "analysis_started", "invoice.pdf");
        log.append("system", "finding_recorded", "a1b2c3d4e5f6");
        log
    }

    #[test]
    fn empty_log_verifies() {
        assert!(AuditLog::new().verify().is_ok());
    }

    #[test]
    fn entries_are_chained() {
        let log = sample_log();
        let entries = log.entries();
        assert_eq!(entries[0].prev_hash, GENESIS_HASH);
        assert_eq!(entries[1].prev_hash, entries[0].hash);
        assert_eq!(entries[2].prev_hash, entries[1].hash);
        assert!(log.verify().is_ok());
    }

    #[test]
    fn editing_an_entry_is_detected() {
        let mut log = sample_log();
        log.entries[1].detail = "other.pdf".into();
        assert!(log.verify().is_err());
    }

    #[test]
    fn removing_an_entry_is_detected() {
        let mut log = sample_log();
        log.entries.remove(1);
        assert!(log.verify().is_err());
    }

    #[test]
    fn json_round_trip_preserves_the_chain() {
        let log = sample_log();
        let restored = AuditLog::from_json(&log.to_json().unwrap()).unwrap();
        assert_eq!(restored.entries(), log.entries());
    }

    #[test]
    fn tampered_json_is_rejected() {
        let json = sample_log()
            .to_json()
            .unwrap()
            .replace("invoice.pdf", "forged.pdf");
        assert!(AuditLog::from_json(&json).is_err());
    }
}

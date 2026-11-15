//! The vault document (the plaintext inside the encrypted envelope) and the
//! three-way merge that keeps a stale device from overwriting newer
//! configuration (ADR-032). Pure: no I/O, no keys.
//!
//! A record is one synced thing (`storage-account:<id>`,
//! `backup-job:<id>`), with a revision counter and a tombstone for
//! deletions. Merging a device's local records with the remote vault uses
//! the last vault this device synced as the common base:
//!
//! - changed on one side only → that side wins;
//! - changed on both sides, to the same value → fine;
//! - changed on both sides differently → the remote value stays and the
//!   local one is kept as a visible conflict copy
//!   (`conflict:<key>:<device>`), never dropped silently;
//! - a deletion against a change → the change wins (no data is lost by a
//!   merge).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DOC_FORMAT: &str = "sotyware-vault-doc";

/// One synced thing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub rev: u64,
    pub updated_at: i64,
    pub device: String,
    pub deleted: bool,
    pub data: Value,
}

impl Record {
    /// Whether two records say the same thing (revision and author aside).
    pub fn same(&self, other: &Record) -> bool {
        self.deleted == other.deleted && (self.deleted || self.data == other.data)
    }
}

/// A device trusted to receive the account key on rotation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedDevice {
    pub public_key: String,
    pub name: String,
    pub added_by: String,
}

/// The vault's plaintext.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Doc {
    pub format: String,
    /// The service revision this document was written as: checked against
    /// the revision the service reports and the highest one this device
    /// saw, so the service cannot replay an older vault unnoticed.
    pub revision: i64,
    pub updated_at: i64,
    pub updated_by: String,
    pub trusted_devices: BTreeMap<String, TrustedDevice>,
    pub records: BTreeMap<String, Record>,
}

impl Doc {
    pub fn empty() -> Self {
        Self {
            format: DOC_FORMAT.into(),
            revision: 0,
            updated_at: 0,
            updated_by: String::new(),
            trusted_devices: BTreeMap::new(),
            records: BTreeMap::new(),
        }
    }

    pub fn check(&self) -> Result<(), String> {
        if self.format != DOC_FORMAT {
            return Err(format!("not a sotyware vault document ({})", self.format));
        }
        Ok(())
    }
}

/// What a merge produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Merged {
    pub records: BTreeMap<String, Record>,
    /// Keys whose local change lost to a different remote change.
    pub conflicts: Vec<String>,
    /// Whether the result differs from the remote records (so it must be
    /// written).
    pub changed: bool,
}

fn equal(a: Option<&Record>, b: Option<&Record>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x.same(y),
        // A tombstone and an absent record say the same.
        (Some(x), None) | (None, Some(x)) => x.deleted,
    }
}

/// Merges `local` (this device's current state, with tombstones for what it
/// deleted since `base`) into `remote`, from the common `base`.
pub fn merge(
    base: &BTreeMap<String, Record>,
    local: &BTreeMap<String, Record>,
    remote: &BTreeMap<String, Record>,
    device: &str,
    now: i64,
) -> Merged {
    let keys: BTreeSet<&String> = base
        .keys()
        .chain(local.keys())
        .chain(remote.keys())
        .collect();
    let mut out = Merged::default();
    for key in keys {
        let (b, l, r) = (base.get(key), local.get(key), remote.get(key));
        let next_rev = [b, l, r].iter().flatten().map(|x| x.rev).max().unwrap_or(0) + 1;
        let stamp = |rec: &Record| Record {
            rev: next_rev,
            updated_at: now,
            device: device.into(),
            ..rec.clone()
        };
        let chosen: Option<Record> = if equal(l, r) {
            r.cloned().or_else(|| l.cloned())
        } else if equal(l, b) {
            // Only the remote side changed.
            r.cloned()
        } else if equal(r, b) {
            // Only this device changed.
            l.map(stamp)
        } else {
            // Both changed, differently.
            match (l, r) {
                (Some(lr), Some(rr)) if lr.deleted && !rr.deleted => Some(rr.clone()),
                (Some(lr), Some(rr)) if rr.deleted && !lr.deleted => Some(stamp(lr)),
                (Some(lr), None) if !lr.deleted => Some(stamp(lr)),
                (None, Some(rr)) => Some(rr.clone()),
                (Some(lr), Some(rr)) => {
                    let copy = format!("conflict:{key}:{device}");
                    out.records.insert(copy.clone(), stamp(lr));
                    out.conflicts.push(key.clone());
                    Some(rr.clone())
                }
                _ => r.cloned(),
            }
        };
        if let Some(c) = chosen {
            out.records.insert(key.clone(), c);
        }
    }
    // Conflict copies already in the remote vault stay until resolved.
    for (k, v) in remote {
        if k.starts_with("conflict:") && !out.records.contains_key(k) {
            out.records.insert(k.clone(), v.clone());
        }
    }
    out.changed = out.records.len() != remote.len()
        || out
            .records
            .iter()
            .any(|(k, v)| remote.get(k).is_none_or(|r| !r.same(v) || r.rev != v.rev));
    out
}

/// Adds tombstones to `local` for every record of `base` this device no
/// longer has (it deleted them since the last sync).
pub fn with_tombstones(
    mut local: BTreeMap<String, Record>,
    base: &BTreeMap<String, Record>,
    device: &str,
    now: i64,
) -> BTreeMap<String, Record> {
    for (k, b) in base {
        if !k.starts_with("conflict:") && !b.deleted && !local.contains_key(k) {
            local.insert(
                k.clone(),
                Record {
                    rev: b.rev,
                    updated_at: now,
                    device: device.into(),
                    deleted: true,
                    data: Value::Null,
                },
            );
        }
    }
    local
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(rev: u64, data: Value) -> Record {
        Record {
            rev,
            updated_at: 1,
            device: "x".into(),
            deleted: false,
            data,
        }
    }

    fn map(items: &[(&str, Record)]) -> BTreeMap<String, Record> {
        items
            .iter()
            .map(|(k, r)| ((*k).to_owned(), r.clone()))
            .collect()
    }

    #[test]
    fn one_sided_changes_win_and_nothing_else_changes() {
        let base = map(&[("a", rec(1, json!(1))), ("b", rec(1, json!(2)))]);
        // Remote changed a; local changed b and added c.
        let remote = map(&[("a", rec(2, json!(10))), ("b", rec(1, json!(2)))]);
        let local = map(&[
            ("a", rec(1, json!(1))),
            ("b", rec(1, json!(20))),
            ("c", rec(0, json!(3))),
        ]);
        let m = merge(&base, &local, &remote, "dev", 5);
        assert_eq!(m.records["a"].data, json!(10));
        assert_eq!(m.records["b"].data, json!(20));
        assert_eq!(m.records["b"].rev, 2);
        assert_eq!(m.records["c"].data, json!(3));
        assert!(m.conflicts.is_empty());
        assert!(m.changed);
        // Nothing local changed: nothing to write.
        let m = merge(&remote, &remote, &remote, "dev", 5);
        assert!(!m.changed);
    }

    #[test]
    fn a_stale_device_never_overwrites_a_newer_change() {
        let base = map(&[("acct", rec(1, json!({"label": "Photos"})))]);
        let remote = map(&[("acct", rec(2, json!({"label": "Photos 2026"})))]);
        // The stale device did not touch it: the newer remote value stays.
        let m = merge(&base, &base, &remote, "stale", 5);
        assert_eq!(m.records["acct"].data, json!({"label": "Photos 2026"}));
        assert!(!m.changed);
    }

    #[test]
    fn concurrent_different_changes_keep_both_values() {
        let base = map(&[("acct", rec(1, json!("v1")))]);
        let remote = map(&[("acct", rec(2, json!("remote")))]);
        let local = map(&[("acct", rec(1, json!("local")))]);
        let m = merge(&base, &local, &remote, "dev", 5);
        assert_eq!(m.records["acct"].data, json!("remote"));
        assert_eq!(m.records["conflict:acct:dev"].data, json!("local"));
        assert_eq!(m.conflicts, ["acct"]);
        // The same change on both sides is no conflict.
        let m = merge(&base, &remote, &remote, "dev", 5);
        assert!(m.conflicts.is_empty());
    }

    #[test]
    fn deletions_propagate_but_never_beat_a_change() {
        let base = map(&[("a", rec(1, json!(1))), ("b", rec(1, json!(2)))]);
        // Locally deleted a (tombstone); remote unchanged → deleted.
        let local = with_tombstones(map(&[("b", rec(1, json!(2)))]), &base, "dev", 5);
        assert!(local["a"].deleted);
        let m = merge(&base, &local, &base, "dev", 5);
        assert!(m.records["a"].deleted);
        // Locally deleted a, remote changed a → the change stays.
        let remote = map(&[("a", rec(2, json!(100))), ("b", rec(1, json!(2)))]);
        let m = merge(&base, &local, &remote, "dev", 5);
        assert!(!m.records["a"].deleted);
        assert_eq!(m.records["a"].data, json!(100));
        // Remote deleted b, locally changed b → the local change stays.
        let mut tomb = rec(2, Value::Null);
        tomb.deleted = true;
        let remote = map(&[("a", rec(1, json!(1))), ("b", tomb)]);
        let local = map(&[("a", rec(1, json!(1))), ("b", rec(1, json!(22)))]);
        let m = merge(&base, &local, &remote, "dev", 5);
        assert_eq!(m.records["b"].data, json!(22));
        assert!(!m.records["b"].deleted);
    }

    #[test]
    fn documents_are_checked() {
        let d = Doc::empty();
        assert_eq!(d.check(), Ok(()));
        let mut bad = d.clone();
        bad.format = "x".into();
        assert!(bad.check().is_err());
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("trustedDevices"));
        assert_eq!(serde_json::from_str::<Doc>(&json).unwrap(), d);
    }
}

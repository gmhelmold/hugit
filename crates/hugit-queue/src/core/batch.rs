//! Batching landable entries into an ordered, DAG-ordered batch.
//!
//! A `Batch` is the unit the union fold operates over. Entries carry their
//! contract identity (`LandableEntry`) plus the engine-internal affected-set
//! (B3's shape) used for disjointness and memoised-check selection.

use crate::core::affected::AffectedSet;
use crate::core::state::EntryState;
use hugit_contracts::LandableEntry;
use std::collections::BTreeSet;

/// Why a batch could not be constructed from a set of entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchError {
    /// Two (or more) entries share the same `order_index`. Queue order is the
    /// load-bearing ⑤ invariant; a duplicate index makes the order ambiguous
    /// (a sort would pick an arbitrary winner), so it is rejected rather than
    /// silently tolerated. Carries the duplicated index.
    DuplicateOrderIndex(u64),
}

impl std::fmt::Display for BatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BatchError::DuplicateOrderIndex(i) => {
                write!(
                    f,
                    "duplicate order_index {i} — queue order would be ambiguous"
                )
            }
        }
    }
}

impl std::error::Error for BatchError {}

/// One landable change inside a batch: its frozen contract identity, its
/// affected check-keys (B3), and its current state-machine state.
#[derive(Debug, Clone)]
pub struct BatchEntry {
    /// Frozen contract identity of the landable change.
    pub landable: LandableEntry,
    /// The check-keys this change affects (B3's shape).
    pub affected: AffectedSet,
    /// Current state-machine state.
    pub state: EntryState,
}

impl BatchEntry {
    /// Create a fresh (`Landable`) batch entry.
    pub fn new(landable: LandableEntry, affected: AffectedSet) -> Self {
        Self {
            landable,
            affected,
            state: EntryState::Landable,
        }
    }

    /// The queue position of this entry (from the frozen contract).
    pub fn order_index(&self) -> u64 {
        self.landable.order_index
    }

    /// The item id of this entry.
    pub fn item_id(&self) -> &str {
        &self.landable.item_id
    }
}

/// An ordered batch of landable entries. Entries are held in strict
/// `order_index` order — the DAG ordering — so that landing always proceeds in
/// queue order and the structural ordering invariant (⑤) holds.
#[derive(Debug, Clone, Default)]
pub struct Batch {
    /// Batch identifier (mirrors `UnionResult.batch_id`).
    pub batch_id: String,
    entries: Vec<BatchEntry>,
}

impl Batch {
    /// Create an empty batch with the given id.
    pub fn new(batch_id: impl Into<String>) -> Self {
        Self {
            batch_id: batch_id.into(),
            entries: Vec::new(),
        }
    }

    /// Build a batch from landable entries paired with their affected-sets,
    /// rejecting a duplicate `order_index`.
    ///
    /// Entries are sorted into strict `order_index` order so that iteration
    /// order *is* queue order — there is no later opportunity to land out of
    /// order. A duplicate `order_index` is refused with
    /// [`BatchError::DuplicateOrderIndex`]: two entries at the same position
    /// make the queue order ambiguous, which would silently undermine ⑤.
    pub fn try_from_entries(
        batch_id: impl Into<String>,
        entries: impl IntoIterator<Item = (LandableEntry, AffectedSet)>,
    ) -> Result<Self, BatchError> {
        let mut entries: Vec<BatchEntry> = entries
            .into_iter()
            .map(|(l, a)| BatchEntry::new(l, a))
            .collect();
        // Reject duplicate order_index before sorting — a duplicate would let a
        // stable sort pick an arbitrary winner and bury the ambiguity.
        let mut seen = BTreeSet::new();
        for e in &entries {
            if !seen.insert(e.order_index()) {
                return Err(BatchError::DuplicateOrderIndex(e.order_index()));
            }
        }
        entries.sort_by_key(|e| e.order_index());
        Ok(Self {
            batch_id: batch_id.into(),
            entries,
        })
    }

    /// Build a batch, panicking on a duplicate `order_index`.
    ///
    /// Convenience over [`Batch::try_from_entries`] for call sites that have
    /// already established unique queue positions (tests, fixtures). A duplicate
    /// is a contract violation, so it panics rather than silently picking a
    /// winner — production paths that accept untrusted ordering should use
    /// [`Batch::try_from_entries`] and handle the error.
    pub fn from_entries(
        batch_id: impl Into<String>,
        entries: impl IntoIterator<Item = (LandableEntry, AffectedSet)>,
    ) -> Self {
        Self::try_from_entries(batch_id, entries)
            .expect("from_entries: duplicate order_index (use try_from_entries to handle)")
    }

    /// The entries in strict queue order.
    pub fn entries(&self) -> &[BatchEntry] {
        &self.entries
    }

    /// Mutable access to entries (still in queue order).
    pub fn entries_mut(&mut self) -> &mut [BatchEntry] {
        &mut self.entries
    }

    /// Number of entries in the batch.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the batch holds no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True iff entries are in strictly increasing `order_index` order (the
    /// DAG-ordering invariant). Always true for a batch built via this module,
    /// asserted by tests as a structural guard.
    pub fn is_queue_ordered(&self) -> bool {
        self.entries
            .windows(2)
            .all(|w| w[0].order_index() < w[1].order_index())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn landable(id: &str, order: u64) -> LandableEntry {
        LandableEntry {
            item_id: id.to_string(),
            intent_id: format!("intent-{id}"),
            tree_hash: format!("tree-{id}"),
            order_index: order,
        }
    }

    #[test]
    fn from_entries_sorts_into_queue_order() {
        let batch = Batch::from_entries(
            "b1",
            [
                (landable("c", 2), AffectedSet::new(["z"])),
                (landable("a", 0), AffectedSet::new(["x"])),
                (landable("b", 1), AffectedSet::new(["y"])),
            ],
        );
        let ids: Vec<&str> = batch.entries().iter().map(|e| e.item_id()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
        assert!(batch.is_queue_ordered());
    }

    #[test]
    fn try_from_entries_rejects_duplicate_order_index() {
        let res = Batch::try_from_entries(
            "dup",
            [
                (landable("a", 0), AffectedSet::new(["x"])),
                (landable("b", 1), AffectedSet::new(["y"])),
                // Same order_index as "b" → ambiguous queue position.
                (landable("c", 1), AffectedSet::new(["z"])),
            ],
        );
        assert_eq!(res.unwrap_err(), BatchError::DuplicateOrderIndex(1));
    }

    #[test]
    #[should_panic(expected = "duplicate order_index")]
    fn from_entries_panics_on_duplicate_order_index() {
        let _ = Batch::from_entries(
            "dup",
            [
                (landable("a", 0), AffectedSet::new(["x"])),
                (landable("b", 0), AffectedSet::new(["y"])),
            ],
        );
    }
}

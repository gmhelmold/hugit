//! Batching landable entries into an ordered, DAG-ordered batch.
//!
//! A `Batch` is the unit the union fold operates over. Entries carry their
//! contract identity (`LandableEntry`) plus the engine-internal affected-set
//! (B3's shape) used for disjointness and memoised-check selection.

use crate::core::affected::AffectedSet;
use crate::core::state::EntryState;
use hugit_contracts::LandableEntry;

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

    /// Build a batch from landable entries paired with their affected-sets.
    /// Entries are sorted into strict `order_index` order on construction so
    /// that iteration order *is* queue order — there is no later opportunity
    /// to land out of order.
    pub fn from_entries(
        batch_id: impl Into<String>,
        entries: impl IntoIterator<Item = (LandableEntry, AffectedSet)>,
    ) -> Self {
        let mut entries: Vec<BatchEntry> = entries
            .into_iter()
            .map(|(l, a)| BatchEntry::new(l, a))
            .collect();
        entries.sort_by_key(|e| e.order_index());
        Self {
            batch_id: batch_id.into(),
            entries,
        }
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
}

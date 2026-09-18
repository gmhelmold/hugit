//! Independent truth-table oracles for HUG-014 / F02.
//! These evaluate sets, not real Git trees or a remote runner.
use hugit_contracts::LandableEntry;
use hugit_queue::core::{
    AffectedSet, Batch, CheckSource, MemoCheck, UnionOutcome, UnionVerdict, evaluate_union,
};

fn batch(ids: &[&str]) -> Batch {
    Batch::from_entries(
        "remainder-safety",
        ids.iter().enumerate().map(|(i, id)| {
            (
                LandableEntry {
                    item_id: (*id).into(),
                    intent_id: format!("intent-{id}"),
                    tree_hash: format!("fixture-{id}"),
                    order_index: i as u64,
                },
                AffectedSet::new(["fixture"]),
            )
        }),
    )
}

struct Oracle<F> {
    rule: F,
    calls: Vec<Vec<String>>,
}
impl<F: Fn(&[&str]) -> bool> MemoCheck for Oracle<F> {
    fn evaluate(&mut self, ids: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
        self.calls
            .push(ids.iter().map(|s| (*s).to_owned()).collect());
        (
            if (self.rule)(ids) {
                UnionVerdict::Green
            } else {
                UnionVerdict::Red
            },
            vec![CheckSource::Executed],
        )
    }
}

#[test]
fn two_independent_conflicts_never_authorize_the_unchecked_remainder() {
    let ids = ["A", "B", "C", "D"];
    let mut oracle = Oracle {
        rule: |s: &[&str]| {
            !(s.contains(&"A") && s.contains(&"B") || s.contains(&"C") && s.contains(&"D"))
        },
        calls: vec![],
    };
    let ev = evaluate_union(&batch(&ids), &mut oracle);
    assert!(
        ev.proceeding.is_empty(),
        "C+D still conflict: {:?}",
        ev.proceeding
    );
    assert!(
        oracle.calls.iter().any(|s| s == &["C", "D"]),
        "exact remainder was never tested"
    );
    assert!(
        ev.outcomes_for_landing(&ids)
            .iter()
            .all(|(id, outcome)| !["C", "D"].contains(id) || *outcome != UnionOutcome::Green)
    );
    assert_eq!(ev.executed_count, oracle.calls.len());
}

#[test]
fn green_remainder_is_evaluated_and_execution_is_accounted() {
    let ids = ["A", "B", "C"];
    let mut oracle = Oracle {
        rule: |s: &[&str]| !(s.contains(&"A") && s.contains(&"B")),
        calls: vec![],
    };
    let ev = evaluate_union(&batch(&ids), &mut oracle);
    assert_eq!(ev.proceeding, ["C"]);
    assert_eq!(oracle.calls.last().unwrap(), &["C"]);
    assert_eq!(ev.executed_count, oracle.calls.len());
    assert_eq!(
        ev.outcomes_for_landing(&ids),
        vec![
            ("A", UnionOutcome::FailingPairMember),
            ("B", UnionOutcome::FailingPairMember),
            ("C", UnionOutcome::Green)
        ]
    );
}

#[test]
fn removing_a_singleton_does_not_assume_monotonicity() {
    let ids = ["A", "B", "C"];
    let mut oracle = Oracle {
        rule: |s: &[&str]| s != ["A"] && s != ["A", "B", "C"] && s != ["B", "C"],
        calls: vec![],
    };
    let ev = evaluate_union(&batch(&ids), &mut oracle);
    assert!(ev.proceeding.is_empty());
    assert_eq!(oracle.calls.last().unwrap(), &["B", "C"]);
    assert_eq!(
        ev.outcomes_for_landing(&ids),
        vec![("A", UnionOutcome::FailingPairMember)]
    );
}

#[test]
fn unlocalized_failure_does_not_accuse_every_member() {
    let ids = ["A", "B", "C"];
    let mut oracle = Oracle {
        rule: |s: &[&str]| s.len() != 3,
        calls: vec![],
    };
    let ev = evaluate_union(&batch(&ids), &mut oracle);
    assert!(ev.proceeding.is_empty());
    assert!(
        ev.outcomes_for_landing(&ids).is_empty(),
        "unknown locus must stay held, not blame all authors"
    );
}

#[test]
fn a_landing_projection_cannot_widen_or_shrink_the_validated_batch() {
    let ids = ["A", "B", "C"];
    let mut oracle = Oracle {
        rule: |_: &[&str]| true,
        calls: vec![],
    };
    let ev = evaluate_union(&batch(&ids), &mut oracle);
    assert!(
        ev.outcomes_for_landing(&["A", "B", "C", "UNTESTED"])
            .is_empty()
    );
    assert!(ev.outcomes_for_landing(&["A", "C"]).is_empty());
    assert!(ev.outcomes_for_landing(&["B", "A", "C"]).is_empty());
}

#[test]
fn all_128_boolean_set_functions_preserve_authorization_soundness() {
    let ids = ["A", "B", "C"];
    for truth in 0u8..128 {
        let green = |s: &[&str]| {
            let mask = s.iter().fold(0usize, |m, id| {
                m | (1 << ids.iter().position(|x| x == id).unwrap())
            });
            mask == 0 || truth & (1 << (mask - 1)) != 0
        };
        let mut oracle = Oracle {
            rule: green,
            calls: vec![],
        };
        let ev = evaluate_union(&batch(&ids), &mut oracle);
        let proceeding: Vec<&str> = ev.proceeding.iter().map(String::as_str).collect();
        if !proceeding.is_empty() {
            assert!(
                green(&proceeding),
                "truth table {truth} authorized a red set {proceeding:?}"
            );
            assert_eq!(
                oracle.calls.last().unwrap(),
                &ev.proceeding,
                "authorization must bind final evaluation"
            );
        }
        let mapped: Vec<&str> = ev
            .outcomes_for_landing(&ids)
            .into_iter()
            .filter_map(|(id, v)| (v == UnionOutcome::Green).then_some(id))
            .collect();
        assert_eq!(
            mapped, proceeding,
            "projection invented a green for truth table {truth}"
        );
        assert_eq!(ev.executed_count, oracle.calls.len());
    }
}

#[test]
fn held_projection_refuses_the_landing_driver_before_mutating_the_batch() {
    let ids = ["A", "B", "C", "D"];
    let mut b = batch(&ids);
    let mut oracle = Oracle {
        rule: |s: &[&str]| {
            !(s.contains(&"A") && s.contains(&"B") || s.contains(&"C") && s.contains(&"D"))
        },
        calls: vec![],
    };
    let ev = evaluate_union(&b, &mut oracle);
    assert_eq!(ev.held, ["C", "D"]);
    assert_eq!(ev.remainder_verdict, Some(UnionVerdict::Red));
    let outcomes = ev.outcomes_for_landing(&ids);
    assert!(hugit_queue::core::land_in_order(&mut b, &outcomes).is_err());
    assert!(
        b.entries()
            .iter()
            .all(|e| e.state == hugit_queue::core::EntryState::Landable)
    );
}

#[test]
fn changing_a_public_summary_does_not_grant_new_authorization() {
    let ids = ["A", "B", "C", "D"];
    let mut oracle = Oracle {
        rule: |s: &[&str]| {
            !(s.contains(&"A") && s.contains(&"B") || s.contains(&"C") && s.contains(&"D"))
        },
        calls: vec![],
    };
    let mut ev = evaluate_union(&batch(&ids), &mut oracle);
    ev.proceeding = vec!["C".into(), "D".into()];
    assert!(ev.validated_proceeding().is_empty());
    assert!(
        ev.outcomes_for_landing(&ids)
            .iter()
            .all(|(_, o)| *o != UnionOutcome::Green)
    );
}

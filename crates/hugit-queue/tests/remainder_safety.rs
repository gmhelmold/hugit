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

#[test]
fn default_budget_bounds_all_hit_diagnostics() {
    let owned: Vec<String> = (0..12).map(|i| format!("member-{i}")).collect();
    let ids: Vec<&str> = owned.iter().map(String::as_str).collect();
    struct AllHits {
        probes: usize,
    }
    impl MemoCheck for AllHits {
        fn evaluate(&mut self, ids: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
            self.probes += 1;
            (
                if ids.len() >= 3 {
                    UnionVerdict::Red
                } else {
                    UnionVerdict::Green
                },
                vec![CheckSource::Hit],
            )
        }
    }
    let mut oracle = AllHits { probes: 0 };
    let ev = evaluate_union(&batch(&ids), &mut oracle);
    assert!(
        oracle.probes <= 64,
        "budget bypassed by cache hits: {}",
        oracle.probes
    );
    assert!(ev.validated_proceeding().is_empty());
    assert!(ev.outcomes_for_landing(&ids).is_empty());
    assert_eq!(ev.executed_count, 0);
}

use hugit_queue::core::union::{EvaluationLimits, EvaluationStop, evaluate_union_with_limits};
use std::time::{Duration, Instant};

fn limits(probes: usize) -> EvaluationLimits {
    EvaluationLimits {
        max_probes: probes,
        timeout: Duration::from_secs(30),
    }
}

#[test]
fn zero_budget_never_calls_the_oracle() {
    let ids = ["A", "B"];
    let mut oracle = Oracle {
        rule: |_: &[&str]| panic!("no budget"),
        calls: vec![],
    };
    let ev = evaluate_union_with_limits(&batch(&ids), &mut oracle, limits(0));
    assert_eq!(ev.stop_reason, Some(EvaluationStop::ProbeBudgetExhausted));
    assert_eq!(ev.verdict, UnionVerdict::Unknown);
    assert_eq!(ev.probe_count, 0);
    assert_eq!(ev.held, ids);
    assert!(ev.validated_proceeding().is_empty());
    assert!(ev.outcomes_for_landing(&ids).is_empty());
}

#[test]
fn remainder_probe_consumes_the_last_budget_slot_or_holds_everything() {
    let ids = ["A", "B", "C"];
    for (ceiling, succeeds) in [(5, false), (6, true)] {
        let mut oracle = Oracle {
            rule: |s: &[&str]| !(s.contains(&"A") && s.contains(&"B")),
            calls: vec![],
        };
        let ev = evaluate_union_with_limits(&batch(&ids), &mut oracle, limits(ceiling));
        assert_eq!(ev.probe_count, ceiling);
        assert_eq!(ev.executed_count, oracle.calls.len());
        assert!(ev.failure.is_some(), "partial diagnosis is retained");
        if succeeds {
            assert_eq!(ev.validated_proceeding(), ["C"]);
            assert!(ev.stop_reason.is_none());
            assert_eq!(oracle.calls.last().unwrap(), &["C"]);
        } else {
            assert_eq!(ev.stop_reason, Some(EvaluationStop::ProbeBudgetExhausted));
            assert_eq!(ev.held, ids);
            assert!(ev.outcomes_for_landing(&ids).is_empty());
            assert!(ev.validated_excluded().is_empty());
        }
    }
}

struct Interrupted {
    calls: usize,
    interrupt_at: usize,
    verdict: UnionVerdict,
    deadlines: Vec<Instant>,
}
impl MemoCheck for Interrupted {
    fn evaluate(&mut self, ids: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
        self.calls += 1;
        let v = if self.calls == self.interrupt_at {
            self.verdict
        } else if ids.contains(&"A") && ids.contains(&"B") {
            UnionVerdict::Red
        } else {
            UnionVerdict::Green
        };
        (v, vec![CheckSource::Hit])
    }
    fn evaluate_before(
        &mut self,
        ids: &[&str],
        deadline: Instant,
    ) -> (UnionVerdict, Vec<CheckSource>) {
        self.deadlines.push(deadline);
        self.evaluate(ids)
    }
}

#[test]
fn unknown_and_infrastructure_at_every_phase_never_accuse_or_authorize() {
    let ids = ["A", "B", "C"];
    for (v, reason) in [
        (UnionVerdict::Unknown, EvaluationStop::Unknown),
        (
            UnionVerdict::InfrastructureFailure,
            EvaluationStop::InfrastructureFailure,
        ),
    ] {
        // Root, singleton, pair diagnosis, and final remainder.
        for phase in [1, 2, 5, 6] {
            let mut oracle = Interrupted {
                calls: 0,
                interrupt_at: phase,
                verdict: v,
                deadlines: vec![],
            };
            let ev = evaluate_union(&batch(&ids), &mut oracle);
            assert_eq!(ev.stop_reason, Some(reason));
            assert_eq!(ev.held, ids);
            assert!(ev.validated_proceeding().is_empty());
            assert!(ev.validated_excluded().is_empty());
            assert!(ev.outcomes_for_landing(&ids).is_empty());
            assert_eq!(ev.probe_count, phase);
            assert_eq!(oracle.calls, phase);
            assert_eq!(
                ev.executed_count, 0,
                "hits cannot become fabricated executions"
            );
            assert!(!ev.execution_count_complete);
            if phase == 1 {
                assert_eq!(ev.verdict, v);
            } else {
                assert_eq!(
                    ev.verdict,
                    UnionVerdict::Red,
                    "retain the observed root verdict"
                );
            }
            if phase == 6 {
                assert!(ev.failure.is_some(), "retain partial pair diagnosis");
            }
        }
    }
}

#[test]
fn a_single_absolute_deadline_is_forwarded_to_all_probes() {
    let mut oracle = Interrupted {
        calls: 0,
        interrupt_at: usize::MAX,
        verdict: UnionVerdict::Unknown,
        deadlines: vec![],
    };
    let ev = evaluate_union(&batch(&["A", "B", "C"]), &mut oracle);
    assert_eq!(ev.probe_count, 6);
    assert_eq!(ev.executed_count, 0);
    assert!(oracle.deadlines.iter().all(|d| *d == oracle.deadlines[0]));
    assert_eq!(ev.validated_proceeding(), ["C"]);
}

#[test]
fn zero_timeout_is_held_without_a_probe() {
    let ids = ["A"];
    let mut oracle = Oracle {
        rule: |_: &[&str]| panic!("deadline expired"),
        calls: vec![],
    };
    let ev = evaluate_union_with_limits(
        &batch(&ids),
        &mut oracle,
        EvaluationLimits {
            max_probes: 64,
            timeout: Duration::ZERO,
        },
    );
    assert_eq!(ev.stop_reason, Some(EvaluationStop::DeadlineExceeded));
    assert_eq!(ev.probe_count, 0);
    assert_eq!(ev.held, ids);
}

#[test]
fn overflowing_deadline_is_refused_not_unlimited() {
    let mut oracle = Oracle {
        rule: |_: &[&str]| panic!("invalid deadline"),
        calls: vec![],
    };
    let ev = evaluate_union_with_limits(
        &batch(&["A"]),
        &mut oracle,
        EvaluationLimits {
            max_probes: 64,
            timeout: Duration::MAX,
        },
    );
    assert_eq!(ev.stop_reason, Some(EvaluationStop::InvalidLimits));
    assert_eq!(ev.probe_count, 0);
    assert!(ev.validated_proceeding().is_empty());
}

#[test]
fn duplicate_member_ids_never_reuse_another_members_result() {
    let mut oracle = Oracle {
        rule: |_: &[&str]| panic!("duplicate ID"),
        calls: vec![],
    };
    let ev = evaluate_union(&batch(&["A", "A"]), &mut oracle);
    assert_eq!(ev.stop_reason, Some(EvaluationStop::InvalidBatch));
    assert_eq!(ev.probe_count, 0);
}

#[test]
fn invalidation_revokes_cloned_results_without_mutating_the_queue() {
    let mut b = batch(&["A", "B"]);
    let mut oracle = Oracle {
        rule: |_: &[&str]| true,
        calls: vec![],
    };
    let mut ev = evaluate_union(&b, &mut oracle);
    let copy = ev.clone();
    assert_eq!(copy.validated_proceeding(), ["A", "B"]);
    ev.invalidate();
    assert!(copy.validated_proceeding().is_empty());
    assert!(copy.outcomes_for_landing(&["A", "B"]).is_empty());
    assert!(
        hugit_queue::core::land_in_order(&mut b, &copy.outcomes_for_landing(&["A", "B"])).is_err()
    );
    assert!(
        b.entries()
            .iter()
            .all(|e| e.state == hugit_queue::core::EntryState::Landable)
    );
    let retry = evaluate_union(&b, &mut oracle);
    assert_eq!(retry.validated_proceeding(), ["A", "B"]);
    assert!(
        copy.validated_proceeding().is_empty(),
        "a retry cannot revive the previous evaluation"
    );
}

#[test]
fn edited_diagnostic_summary_cannot_exclude_an_innocent_member() {
    let ids = ["A", "B", "C", "D"];
    let mut oracle = Oracle {
        rule: |s: &[&str]| {
            !(s.contains(&"A") && s.contains(&"B") || s.contains(&"C") && s.contains(&"D"))
        },
        calls: vec![],
    };
    let mut ev = evaluate_union(&batch(&ids), &mut oracle);
    let expected = ev.outcomes_for_landing(&ids);
    ev.failure = Some(hugit_queue::core::FailureLocus::SingleItem("C".into()));
    ev.held.clear();
    ev.verdict = UnionVerdict::Green;
    ev.proceeding = vec!["D".into()];
    assert_eq!(ev.outcomes_for_landing(&ids), expected);
    assert_eq!(ev.validated_excluded(), ["A", "B"]);
}

#[test]
fn empty_batch_never_probes_or_grants_a_member() {
    let mut oracle = Oracle {
        rule: |_: &[&str]| panic!("empty batch"),
        calls: vec![],
    };
    let ev = evaluate_union(&batch(&[]), &mut oracle);
    assert_eq!(ev.probe_count, 0);
    assert!(ev.validated_proceeding().is_empty());
    assert!(ev.outcomes_for_landing(&["UNTESTED"]).is_empty());
}

#[test]
fn all_boolean_set_functions_remain_sound_at_every_small_probe_budget() {
    let ids = ["A", "B", "C"];
    for truth in 0u8..128 {
        let green = |set: &[&str]| {
            let mask = set.iter().fold(0usize, |m, id| {
                m | (1 << ids.iter().position(|x| x == id).unwrap())
            });
            mask == 0 || truth & (1 << (mask - 1)) != 0
        };
        for ceiling in 0..=8 {
            let mut oracle = Oracle {
                rule: green,
                calls: vec![],
            };
            let ev = evaluate_union_with_limits(&batch(&ids), &mut oracle, limits(ceiling));
            assert!(oracle.calls.len() <= ceiling);
            assert_eq!(ev.probe_count, oracle.calls.len());
            if ev.stop_reason.is_some() {
                assert_eq!(ev.held, ids);
                assert!(ev.outcomes_for_landing(&ids).is_empty());
            } else {
                let permitted: Vec<&str> = ev
                    .validated_proceeding()
                    .iter()
                    .map(String::as_str)
                    .collect();
                if !permitted.is_empty() {
                    assert!(green(&permitted));
                    assert_eq!(oracle.calls.last().unwrap(), ev.validated_proceeding());
                }
            }
        }
    }
}

#[test]
fn caller_modified_queue_order_is_refused_before_any_probe() {
    let mut b = batch(&["A", "B"]);
    b.entries_mut()[1].landable.order_index = 0;
    let mut oracle = Oracle {
        rule: |_: &[&str]| panic!("ambiguous ordering"),
        calls: vec![],
    };
    let ev = evaluate_union(&b, &mut oracle);
    assert_eq!(ev.stop_reason, Some(EvaluationStop::InvalidBatch));
    assert_eq!(ev.probe_count, 0);
}

// HUG-014: a bound result is not reusable under different contents or context.
use hugit_queue::core::union::{EvaluationContext, evaluate_union_in_context};

fn context() -> EvaluationContext {
    EvaluationContext {
        operation_id: "op-1".into(),
        base_id: "fixture-base-1".into(),
        config_id: "fixture-config-1".into(),
        scope: "simulation_only".into(),
    }
}

#[test]
fn bound_result_records_each_probe_and_authorizes_the_exact_remainder() {
    let b = batch(&["A", "B", "C"]);
    let mut oracle = Oracle {
        rule: |s: &[&str]| !(s.contains(&"A") && s.contains(&"B")),
        calls: vec![],
    };
    let ev = evaluate_union_in_context(&b, &mut oracle, limits(64), context());
    let auth = ev.authorize_for(&b, &context()).unwrap();
    assert_eq!(auth.proceeding, ["C"]);
    assert_eq!(auth.excluded, ["A", "B"]);
    let binding = ev.binding().unwrap();
    assert_eq!(binding.context(), &context());
    assert_eq!(binding.probes().len(), ev.probe_count);
    assert_eq!(
        binding
            .probes()
            .iter()
            .map(|p| p.members.clone())
            .collect::<Vec<_>>(),
        oracle.calls
    );
    assert_eq!(binding.probes().last().unwrap().members, ["C"]);
    assert_eq!(
        binding.probes().last().unwrap().returned_verdict,
        UnionVerdict::Green
    );
    // Compatibility entry points cannot strip the context requirement.
    assert!(ev.validated_proceeding().is_empty());
    assert!(ev.validated_excluded().is_empty());
    assert!(ev.outcomes_for_landing(&["A", "B", "C"]).is_empty());
}

#[test]
fn same_member_ids_with_changed_content_state_or_affected_set_are_not_authorized() {
    let b = batch(&["A", "B"]);
    let mut oracle = Oracle {
        rule: |_: &[&str]| true,
        calls: vec![],
    };
    let ev = evaluate_union_in_context(&b, &mut oracle, limits(64), context());
    for dimension in 0..7 {
        let mut changed = b.clone();
        match dimension {
            0 => changed.entries_mut()[0].landable.tree_hash = "new-tree".into(),
            1 => changed.entries_mut()[0].landable.intent_id = "new-intent".into(),
            2 => changed.entries_mut()[0].affected = AffectedSet::new(["new-check"]),
            3 => changed.entries_mut()[0].state = hugit_queue::core::EntryState::Landed,
            4 => changed.entries_mut()[1].landable.order_index += 10,
            5 => changed.batch_id = "other-batch".into(),
            6 => changed.entries_mut().swap(0, 1),
            _ => unreachable!(),
        }
        assert!(
            ev.authorize_for(&changed, &context()).is_none(),
            "dimension {dimension}"
        );
    }
    assert_eq!(
        ev.authorize_for(&b, &context()).unwrap().proceeding,
        ["A", "B"]
    );
    assert_eq!(oracle.calls.len(), 1);
}

#[test]
fn changed_base_configuration_operation_or_scope_cannot_reuse_bound_authorization() {
    let b = batch(&["A", "B"]);
    let mut oracle = Oracle {
        rule: |_: &[&str]| true,
        calls: vec![],
    };
    let ev = evaluate_union_in_context(&b, &mut oracle, limits(64), context());
    for dimension in 0..4 {
        let mut changed = context();
        let field = match dimension {
            0 => &mut changed.base_id,
            1 => &mut changed.config_id,
            2 => &mut changed.operation_id,
            _ => &mut changed.scope,
        };
        *field = "different".into();
        assert!(ev.authorize_for(&b, &changed).is_none());
    }
    let mut legacy = Oracle {
        rule: |_: &[&str]| true,
        calls: vec![],
    };
    assert!(
        evaluate_union(&b, &mut legacy)
            .authorize_for(&b, &context())
            .is_none()
    );
}

#[test]
fn invalid_context_admits_no_probe_and_no_authorization() {
    for dimension in 0..4 {
        let mut ctx = context();
        let field = match dimension {
            0 => &mut ctx.base_id,
            1 => &mut ctx.config_id,
            2 => &mut ctx.operation_id,
            _ => &mut ctx.scope,
        };
        field.clear();
        let b = batch(&["A"]);
        let mut oracle = Oracle {
            rule: |_: &[&str]| panic!("invalid context"),
            calls: vec![],
        };
        let ev = evaluate_union_in_context(&b, &mut oracle, limits(64), ctx.clone());
        assert_eq!(ev.stop_reason, Some(EvaluationStop::InvalidContext));
        assert_eq!(ev.probe_count, 0);
        assert!(ev.binding().unwrap().probes().is_empty());
        assert!(ev.authorize_for(&b, &ctx).is_none());
    }
}

#[test]
fn bound_invalidation_reaches_clones_and_retry_does_not_revive_old_operation() {
    let b = batch(&["A"]);
    let mut oracle = Oracle {
        rule: |_: &[&str]| true,
        calls: vec![],
    };
    let mut first = evaluate_union_in_context(&b, &mut oracle, limits(64), context());
    let copy = first.clone();
    first.invalidate();
    let mut next = context();
    next.operation_id = "op-2".into();
    let retry = evaluate_union_in_context(&b, &mut oracle, limits(64), next.clone());
    assert!(copy.authorize_for(&b, &context()).is_none());
    assert!(copy.authorize_for(&b, &next).is_none());
    assert!(retry.authorize_for(&b, &context()).is_none());
    assert_eq!(retry.authorize_for(&b, &next).unwrap().proceeding, ["A"]);
}

#[test]
fn every_scoped_probe_receives_the_same_context_and_absolute_deadline() {
    struct Scoped {
        received: Vec<(Vec<String>, EvaluationContext, std::time::Instant)>,
    }
    impl MemoCheck for Scoped {
        fn evaluate(&mut self, _: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
            panic!("unscoped")
        }
        fn evaluate_in_context(
            &mut self,
            ids: &[&str],
            deadline: std::time::Instant,
            ctx: &EvaluationContext,
        ) -> (UnionVerdict, Vec<CheckSource>) {
            self.received.push((
                ids.iter().map(|s| (*s).to_owned()).collect(),
                ctx.clone(),
                deadline,
            ));
            (
                if ids.contains(&"A") && ids.contains(&"B") {
                    UnionVerdict::Red
                } else {
                    UnionVerdict::Green
                },
                vec![CheckSource::Hit],
            )
        }
    }
    let b = batch(&["A", "B", "C"]);
    let mut oracle = Scoped { received: vec![] };
    let ev = evaluate_union_in_context(&b, &mut oracle, limits(64), context());
    assert_eq!(ev.probe_count, 6);
    assert_eq!(ev.binding().unwrap().probes().len(), 6);
    assert!(
        oracle
            .received
            .iter()
            .all(|(_, ctx, deadline)| ctx == &context() && *deadline == oracle.received[0].2)
    );
    assert_eq!(oracle.received.last().unwrap().0, ["C"]);
}

#[test]
fn bound_result_never_authorizes_inconclusive_budget_or_infrastructure() {
    let b = batch(&["A", "B"]);
    let mut zero = Oracle {
        rule: |_: &[&str]| panic!("zero budget"),
        calls: vec![],
    };
    let ev = evaluate_union_in_context(&b, &mut zero, limits(0), context());
    assert!(ev.authorize_for(&b, &context()).is_none());
    struct Infra;
    impl MemoCheck for Infra {
        fn evaluate(&mut self, _: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
            (UnionVerdict::InfrastructureFailure, vec![])
        }
    }
    let ev = evaluate_union_in_context(&b, &mut Infra, limits(64), context());
    assert!(ev.authorize_for(&b, &context()).is_none());
    assert_eq!(
        ev.binding().unwrap().probes()[0].returned_verdict,
        UnionVerdict::InfrastructureFailure
    );
}

#[test]
fn bound_truth_tables_keep_all_authorization_tied_to_the_last_green_set() {
    let ids = ["A", "B", "C"];
    let b = batch(&ids);
    for truth in 0u8..128 {
        let green = |set: &[&str]| {
            let mask = set.iter().fold(0usize, |m, id| {
                m | (1 << ids.iter().position(|x| x == id).unwrap())
            });
            mask == 0 || truth & (1 << (mask - 1)) != 0
        };
        for ceiling in 0..=8 {
            let mut oracle = Oracle {
                rule: green,
                calls: vec![],
            };
            let ev = evaluate_union_in_context(&b, &mut oracle, limits(ceiling), context());
            assert_eq!(ev.binding().unwrap().probes().len(), ev.probe_count);
            assert!(ev.probe_count <= ceiling);
            if ev.stop_reason.is_some() {
                assert!(ev.authorize_for(&b, &context()).is_none());
            } else {
                let auth = ev.authorize_for(&b, &context()).unwrap();
                if !auth.proceeding.is_empty() {
                    let set: Vec<&str> = auth.proceeding.iter().map(String::as_str).collect();
                    assert!(green(&set), "truth={truth} budget={ceiling}");
                    assert_eq!(
                        ev.binding().unwrap().probes().last().unwrap().members,
                        auth.proceeding
                    );
                    assert_eq!(
                        ev.binding()
                            .unwrap()
                            .probes()
                            .last()
                            .unwrap()
                            .returned_verdict,
                        UnionVerdict::Green
                    );
                }
            }
        }
    }
}

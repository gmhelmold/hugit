//! Regenerative-rebase **landing gate** (WP-D12).
//!
//! C4 owns the regen DRIVERS (the execution that produces a regenerated
//! derived file). D12 owns the GATE that authorises a regen to **land** as a
//! revision. The gate is the decision surface; it never executes a driver.
//!
//! ## Charter (decomposition v2.0 D12①–⑤)
//! - **① opt-in scope only** — a regen runs ONLY on the opt-in scope from
//!   [`RegenGate::optin_scope`]; a non-opted repo NEVER regens.
//! - **② both preconditions, fail-CLOSED** — a regen lands ONLY if acceptance
//!   re-passes AND a FRESH INDEPENDENT adversarial verdict APPROVES.
//! - **③ missing/failing either → BLOCKED + reported.**
//! - **④ provenance closure** — every regen is its OWN auditable revision; its
//!   [`AttestationChain`] RECORDS the authorising gate-verdict ref ("this regen
//!   was permitted because report-vX passed").
//! - **⑤ anti-smuggling** — a file is "derived" ONLY if the regeneration
//!   command DETERMINISTICALLY produces it from sources. A false "derived"
//!   declaration to bypass the gate is BLOCKED + AUDITED.
//!
//! The gate is **fail-closed**: any missing precondition, any independence
//! defect, any non-derived file masquerading as derived → BLOCKED, with an
//! auditable [`EventRecord`].
//!
//! Consumes (never owns): the D7 [`VerdictObject`] (the independent verdict)
//! and the frozen [`RegenGate`]/[`AttestationChain`]/[`EventRecord`] contracts.

use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signer, SigningKey};
use hugit_contracts::{AttestationChain, EventRecord, RegenGate, Verdict, VerdictObject};
use hugit_refstore::{attestation_sig_preimage, canonical_json, compute_this_hash};

use crate::regen::driver::{DerivedClass, classify};

// ── audit event kinds ───────────────────────────────────────────────────────

/// `EventRecord.kind` for a regen that the gate AUTHORISED to land (④).
pub const KIND_REGEN_LANDED: &str = "regen.gate.landed";
/// `EventRecord.kind` for a regen the gate BLOCKED (③⑤).
pub const KIND_REGEN_BLOCKED: &str = "regen.gate.blocked";

// ── decision surface ────────────────────────────────────────────────────────

/// Why the gate blocked a regen. Each variant is auditable (③⑤).
#[derive(Debug, Clone, PartialEq)]
pub enum BlockReason {
    /// ① the repo is outside the opt-in scope — regen must not run at all.
    NotOptedIn {
        /// The scope the regen request targeted.
        requested: String,
        /// The opt-in scope the gate is configured for.
        optin_scope: String,
    },
    /// ② / ③ acceptance did NOT re-pass.
    RepassFailed,
    /// ② / ③ no independent verdict was supplied (`indep_verdict` empty /
    /// the `VerdictObject` is absent).
    VerdictMissing,
    /// ② / ③ the independent verdict did not APPROVE.
    VerdictNotApproved {
        /// The non-approving outcome actually produced.
        got: Verdict,
    },
    /// ② the supplied verdict is NOT independent of the regen it judges
    /// (circular verification: same model, or it judges a different tree, or
    /// the gate's `indep_verdict` ref does not resolve to this verdict).
    VerdictNotIndependent {
        /// Human-readable description of the independence defect.
        detail: String,
    },
    /// ⑤ anti-smuggling: a file declared "derived" is NOT provably derived
    /// (no recognised derived class, or its regeneration is not deterministic).
    FalseDerivedDeclaration {
        /// The path that was falsely declared derived.
        path: String,
        /// Why the declaration is false.
        detail: String,
    },
}

impl BlockReason {
    /// A stable machine code for this reason (used in the audit payload).
    pub fn code(&self) -> &'static str {
        match self {
            BlockReason::NotOptedIn { .. } => "not_opted_in",
            BlockReason::RepassFailed => "repass_failed",
            BlockReason::VerdictMissing => "verdict_missing",
            BlockReason::VerdictNotApproved { .. } => "verdict_not_approved",
            BlockReason::VerdictNotIndependent { .. } => "verdict_not_independent",
            BlockReason::FalseDerivedDeclaration { .. } => "false_derived_declaration",
        }
    }
}

/// The gate's decision for a regen-landing request.
#[derive(Debug, Clone, PartialEq)]
pub enum GateDecision {
    /// The regen is AUTHORISED to land as its own revision (②④).
    ///
    /// Carries the per-regen [`AttestationChain`] whose principal-chain RECORDS
    /// the authorising gate-verdict ref (provenance closure, ④).
    Land {
        /// The auditable attestation for this regen revision.
        attestation: AttestationChain,
        /// The audit event for the land (kind = [`KIND_REGEN_LANDED`]).
        audit: EventRecord,
    },
    /// The regen is BLOCKED; carries the reason and an audit event (③⑤).
    Blocked {
        /// Why the regen was blocked.
        reason: BlockReason,
        /// The audit event for the block (kind = [`KIND_REGEN_BLOCKED`]).
        audit: EventRecord,
    },
}

impl GateDecision {
    /// `true` iff the regen was authorised to land.
    pub fn is_landed(&self) -> bool {
        matches!(self, GateDecision::Land { .. })
    }

    /// `true` iff the regen was blocked.
    pub fn is_blocked(&self) -> bool {
        matches!(self, GateDecision::Blocked { .. })
    }

    /// The audit [`EventRecord`] produced for this decision — present for BOTH
    /// land and block (every decision is auditable, ③④⑤).
    pub fn audit(&self) -> &EventRecord {
        match self {
            GateDecision::Land { audit, .. } => audit,
            GateDecision::Blocked { audit, .. } => audit,
        }
    }

    /// The block reason, if this decision is a block.
    pub fn block_reason(&self) -> Option<&BlockReason> {
        match self {
            GateDecision::Blocked { reason, .. } => Some(reason),
            GateDecision::Land { .. } => None,
        }
    }

    /// The per-regen attestation, if this decision authorised a land.
    pub fn attestation(&self) -> Option<&AttestationChain> {
        match self {
            GateDecision::Land { attestation, .. } => Some(attestation),
            GateDecision::Blocked { .. } => None,
        }
    }
}

// ── the regen-landing request ───────────────────────────────────────────────

/// A claim that a single file is "derived" and therefore eligible for the
/// regen path (subject to the anti-smuggling check, ⑤).
#[derive(Debug, Clone)]
pub struct DerivedClaim<'a> {
    /// The path being declared derived.
    pub path: &'a Path,
    /// The deterministic-reproduction witness: `true` iff running the
    /// regeneration command on the sources reproduces this exact file
    /// byte-for-byte. A claim is only honoured when this is `true` AND the
    /// path classifies into a recognised [`DerivedClass`] (⑤).
    pub deterministically_reproduced: bool,
}

/// Everything the gate needs to decide one regen-landing.
///
/// The gate IMPLEMENTS the mechanics of the frozen [`RegenGate`]; it does not
/// re-decide the contract's shape.
#[derive(Debug, Clone)]
pub struct RegenRequest<'a> {
    /// The scope this regen targets (e.g. the repo slug).
    pub requested_scope: String,
    /// The model that PRODUCED the regen revision under judgement. Used to
    /// enforce independence of the verdict (② — distinct model required).
    pub regen_model: String,
    /// Content-addressed ref of the workspace tree the regen produced. The
    /// independent verdict MUST judge this same tree.
    pub regen_tree: String,
    /// The files declared derived for this regen (anti-smuggling, ⑤).
    pub derived_claims: Vec<DerivedClaim<'a>>,
}

// ── the gate ────────────────────────────────────────────────────────────────

/// The regenerative-rebase landing gate (D12).
///
/// Constructed from the frozen [`RegenGate`] contract plus the per-regen
/// context; [`Gate::decide`] runs the fail-closed state machine.
#[derive(Debug, Clone)]
pub struct Gate {
    gate: RegenGate,
}

impl Gate {
    /// Build a gate from its frozen [`RegenGate`] contract.
    pub fn new(gate: RegenGate) -> Self {
        Self { gate }
    }

    /// The frozen contract backing this gate.
    pub fn contract(&self) -> &RegenGate {
        &self.gate
    }

    /// `true` iff `requested_scope` is inside this gate's opt-in scope (①).
    ///
    /// `"*"` opts in everything; otherwise an exact match is required. A
    /// non-opted repo is NEVER regenerated.
    pub fn is_opted_in(&self, requested_scope: &str) -> bool {
        self.gate.optin_scope == "*" || self.gate.optin_scope == requested_scope
    }

    /// Decide a single regen-landing request, fail-closed.
    ///
    /// The verdict (`indep`) is the FRESH INDEPENDENT adversarial verdict from a
    /// D7 panel; `None` models a missing verdict (③).
    ///
    /// `prev_hash` is the chain head this decision's audit event extends (the
    /// `this_hash` of the previous event, or
    /// [`hugit_refstore::GENESIS_PREV_HASH`] for the first). The audit event's
    /// `this_hash` is computed via the canonical
    /// [`hugit_refstore::compute_this_hash`] over the canonical-JSON payload, so
    /// gate events chain and verify against the D1 log like every other emitter
    /// (R1). `signing_key` signs the per-regen attestation (item ④) over the
    /// frozen [`hugit_refstore::attestation_sig_preimage`]; a verifier checks it
    /// with the public key alone.
    ///
    /// Evaluation order (each defect short-circuits to a [`BlockReason`]):
    /// 1. ① opt-in scope — non-opted → `NotOptedIn`.
    /// 2. ⑤ anti-smuggling — at least one derived claim is REQUIRED, and every
    ///    declared-derived file must be provably derived, else
    ///    `FalseDerivedDeclaration` (an empty claim set can no longer bypass the
    ///    anti-smuggling check).
    /// 3. ② acceptance re-pass — `repass == false` → `RepassFailed`.
    /// 4. ② independent verdict present, independent, and APPROVE — else
    ///    `VerdictMissing` / `VerdictNotIndependent` / `VerdictNotApproved`.
    ///
    /// On success: ④ a per-regen SIGNED [`AttestationChain`] whose principal-chain
    /// RECORDS the authorising gate-verdict ref, plus a chained land
    /// [`EventRecord`].
    pub fn decide(
        &self,
        req: &RegenRequest<'_>,
        indep: Option<&VerdictObject>,
        seq: u64,
        recorded_at: u64,
        prev_hash: &str,
        signing_key: &SigningKey,
    ) -> GateDecision {
        // ── ① opt-in scope only ────────────────────────────────────────────
        if !self.is_opted_in(&req.requested_scope) {
            return self.block(
                BlockReason::NotOptedIn {
                    requested: req.requested_scope.clone(),
                    optin_scope: self.gate.optin_scope.clone(),
                },
                seq,
                recorded_at,
                prev_hash,
            );
        }

        // ── ⑤ anti-smuggling: every declared-derived file must be provably so ─
        if let Some(reason) = self.anti_smuggling(req) {
            return self.block(reason, seq, recorded_at, prev_hash);
        }

        // ── ② acceptance re-pass (fail-closed) ──────────────────────────────
        if !self.gate.repass {
            return self.block(BlockReason::RepassFailed, seq, recorded_at, prev_hash);
        }

        // ── ② fresh INDEPENDENT adversarial verdict, APPROVE ────────────────
        let verdict = match indep {
            Some(v) => v,
            None => return self.block(BlockReason::VerdictMissing, seq, recorded_at, prev_hash),
        };
        // An empty `indep_verdict` ref means the gate has no verdict bound.
        if self.gate.indep_verdict.trim().is_empty() {
            return self.block(BlockReason::VerdictMissing, seq, recorded_at, prev_hash);
        }
        if let Some(detail) = self.independence_defect(req, verdict) {
            return self.block(
                BlockReason::VerdictNotIndependent { detail },
                seq,
                recorded_at,
                prev_hash,
            );
        }
        if verdict.verdict != Verdict::Approve {
            return self.block(
                BlockReason::VerdictNotApproved {
                    got: verdict.verdict.clone(),
                },
                seq,
                recorded_at,
                prev_hash,
            );
        }

        // ── ④ provenance closure: land as its own auditable revision ────────
        self.land(req, seq, recorded_at, prev_hash, signing_key)
    }

    // ── ⑤ anti-smuggling predicate ──────────────────────────────────────────

    /// A file is honoured as derived ONLY when it classifies into a recognised
    /// [`DerivedClass`] (via C4's `classify`) AND its regeneration is witnessed
    /// to deterministically reproduce it. Any other "derived" declaration is a
    /// smuggling attempt → block + audit (⑤).
    ///
    /// An EMPTY claim set is itself a smuggling attempt: a regen with no proven-
    /// derived file has nothing the gate is entitled to land via the regen path,
    /// so it must NOT slip through with the anti-smuggling check vacuously
    /// satisfied (brutal-review §hugit-checks empty-claims bypass).
    fn anti_smuggling(&self, req: &RegenRequest<'_>) -> Option<BlockReason> {
        if req.derived_claims.is_empty() {
            return Some(BlockReason::FalseDerivedDeclaration {
                path: "<none>".to_string(),
                detail: "regen declared no derived files — an empty claim set cannot \
                         satisfy the anti-smuggling check (nothing is provably derived)"
                    .to_string(),
            });
        }
        for claim in &req.derived_claims {
            let class: Option<DerivedClass> = classify(claim.path);
            if class.is_none() {
                return Some(BlockReason::FalseDerivedDeclaration {
                    path: claim.path.display().to_string(),
                    detail:
                        "path does not classify into any derived class (not a lockfile/codegen/snapshot)"
                            .to_string(),
                });
            }
            if !claim.deterministically_reproduced {
                return Some(BlockReason::FalseDerivedDeclaration {
                    path: claim.path.display().to_string(),
                    detail: "regeneration command does not deterministically reproduce the file from sources".to_string(),
                });
            }
        }
        None
    }

    // ── ② independence predicate ─────────────────────────────────────────────

    /// The verdict must be genuinely independent of the regen it judges:
    /// distinct model (no self-grading), it must judge the SAME tree the regen
    /// produced, and the gate's bound `indep_verdict` ref must resolve to this
    /// verdict (the gate cannot point at one verdict and be handed another).
    fn independence_defect(
        &self,
        req: &RegenRequest<'_>,
        verdict: &VerdictObject,
    ) -> Option<String> {
        if verdict.model == req.regen_model {
            return Some(format!(
                "verdict produced by the same model as the regen (`{}`): not independent",
                req.regen_model
            ));
        }
        if verdict.tree_hash != req.regen_tree {
            return Some(format!(
                "verdict judges tree `{}` but the regen produced tree `{}`",
                verdict.tree_hash, req.regen_tree
            ));
        }
        // The gate's bound ref must address this very verdict via its content
        // anchor (`tree_hash`). The `intent` field is an unauthenticated,
        // regen-controlled label — resolving on it would let a regen point the
        // gate's bound ref at a verdict that judges a DIFFERENT tree, defeating
        // independence (brutal-review §hugit-checks intent-resolution bypass).
        // Resolution is by `tree_hash` ONLY.
        if self.gate.indep_verdict != verdict.tree_hash {
            return Some(format!(
                "gate.indep_verdict ref `{}` does not resolve to the supplied verdict (by tree_hash)",
                self.gate.indep_verdict
            ));
        }
        None
    }

    // ── ④ land: own auditable revision with gate-verdict ref ─────────────────

    fn land(
        &self,
        req: &RegenRequest<'_>,
        seq: u64,
        recorded_at: u64,
        prev_hash: &str,
        signing_key: &SigningKey,
    ) -> GateDecision {
        // The authorising gate-verdict ref — "this regen was permitted because
        // report-vX passed" (provenance closure, ④).
        let verdict_ref = self.gate.indep_verdict.clone();
        // Build the unsigned attestation, then SIGN it over the frozen
        // pre-image so a verifier can confirm provenance with the public key
        // alone (an empty `sig` is no longer accepted, item ④).
        let unsigned = AttestationChain {
            tree: req.regen_tree.clone(),
            def: "regen.gate".to_string(),
            runner: "hugit-checks/regen/gate".to_string(),
            model: req.regen_model.clone(),
            // The principal chain RECORDS the authorising gate-verdict ref as a
            // first-class link: the gate, then the verdict that authorised it.
            principal: vec![
                "regen.gate".to_string(),
                format!("gate-verdict:{verdict_ref}"),
            ],
            sig: String::new(),
        };
        let attestation = sign_attestation(signing_key, unsigned);

        let payload = canonical_payload(&serde_json::json!({
            "scope": req.requested_scope,
            "tree": req.regen_tree,
            "verdict_ref": verdict_ref,
            "verdict_outcome": "approve",
        }));
        let audit = self.audit_event(
            seq,
            prev_hash,
            KIND_REGEN_LANDED,
            vec!["regen.gate".to_string()],
            payload,
            recorded_at,
        );
        GateDecision::Land { attestation, audit }
    }

    // ── ③⑤ block: auditable refusal ──────────────────────────────────────────

    fn block(
        &self,
        reason: BlockReason,
        seq: u64,
        recorded_at: u64,
        prev_hash: &str,
    ) -> GateDecision {
        let payload = canonical_payload(&serde_json::json!({
            "reason": reason.code(),
            "scope": self.gate.optin_scope,
        }));
        let audit = self.audit_event(
            seq,
            prev_hash,
            KIND_REGEN_BLOCKED,
            vec!["regen.gate".to_string()],
            payload,
            recorded_at,
        );
        GateDecision::Blocked { reason, audit }
    }

    /// Build a fully-chained [`EventRecord`] whose `this_hash` is the canonical
    /// [`compute_this_hash`] over `(prev_hash, kind, principal_chain, payload,
    /// seq)` — the single source of truth (R1). The producer and any verifier
    /// recompute the identical digest; gate events therefore chain and verify
    /// against the D1 log like every other emitter.
    fn audit_event(
        &self,
        seq: u64,
        prev_hash: &str,
        kind: &str,
        principal_chain: Vec<String>,
        payload: String,
        recorded_at: u64,
    ) -> EventRecord {
        let this_hash = compute_this_hash(prev_hash, kind, &principal_chain, &payload, seq);
        EventRecord {
            seq,
            prev_hash: prev_hash.to_string(),
            this_hash,
            kind: kind.to_string(),
            principal_chain,
            payload,
            recorded_at,
        }
    }
}

/// Sign an [`AttestationChain`] over the frozen
/// [`attestation_sig_preimage`] (`LP(tree) ‖ LP(def) ‖ LP(runner) ‖ LP(model) ‖
/// VEC(principal)`), returning a chain whose `sig` is the base64 Ed25519
/// signature. The input `sig` is ignored and replaced. Only the producer holds
/// the private key; a verifier checks `sig` with the public key alone (item ④).
fn sign_attestation(signing_key: &SigningKey, chain: AttestationChain) -> AttestationChain {
    let preimage = attestation_sig_preimage(
        &chain.tree,
        &chain.def,
        &chain.runner,
        &chain.model,
        &chain.principal,
    );
    let sig = signing_key.sign(&preimage);
    AttestationChain {
        sig: B64.encode(sig.to_bytes()),
        ..chain
    }
}

/// Render `value` as canonical JSON (sorted keys, no insignificant whitespace)
/// for an audit payload, so equal logical events hash identically (R1). The
/// `serde_json::json!` value is already valid JSON, so canonicalisation never
/// fails; the unreachable fallback emits the compact form rather than panic.
fn canonical_payload(value: &serde_json::Value) -> String {
    let compact = value.to_string();
    canonical_json(&compact).unwrap_or(compact)
}

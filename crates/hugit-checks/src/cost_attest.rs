//! The off-box **cost-attestation verdict** — the CONSUME side of the cost-killer
//! (WP-1 propagation + WP-2 verdict).
//!
//! [`crate::runner::dispatch::dispatch_attest_offbox`] now carries the fabric's
//! `intent_metrics_sig` (+ `fabric_key_id` + the `lease_id` verify binding) OUT on
//! the [`DispatchOutcome`]. This module turns that into a fail-closed typed verdict:
//! is the finalized per-job COST genuinely attested (a present sig that VERIFIES),
//! or not (and precisely why)? It computes ONLY the verdict — it renders/emits
//! nothing. Lighting a `✓ cas:` marker or an `/insights` cost is owner-gated on a
//! real non-zero cost source and lives elsewhere.
//!
//! ## The honesty law (load-bearing)
//!
//! An [`CostAttestation::Attested`] is produced ONLY when a PRESENT
//! `intent_metrics_sig` verifies under the configured fabric pubkey for this
//! lease + tenant + finalized metrics. Every other case is
//! [`CostAttestation::Unattested`] carrying the reason — never a fabricated pass:
//!   - no fabric pubkey/tenant configured ⇒ [`UnattestedReason::NoPubkey`];
//!   - the close carried no sig (fabric emission off) ⇒ [`UnattestedReason::EmissionOff`];
//!   - a sig WAS present but did NOT verify ⇒ [`UnattestedReason::SigInvalid`] — a HARD
//!     signal (a bad/hostile fabric or a MITM), returned DISTINCTLY so the caller can
//!     refuse to claim the cost loudly rather than silently degrade to "unattested".
//!
//! ## The pubkey/tenant seam (config, no live fetch)
//!
//! The verifier needs the fabric pubkey + the exact tenant-binding string. They
//! come from [`FabricAttestConfig`] (env/config injected), NEVER a hardcoded key
//! and NEVER a live fetch here — an absent config is honestly `NoPubkey`, never a
//! fabricated `Attested`.

use crate::attest_v2::verify_intent_metrics_sig;
use crate::runner::dispatch::DispatchOutcome;
use crate::runner::metrics::RunnerJobMetrics;

/// The fabric attestation config: the ed25519 public key (standard-base64, the wire
/// form [`verify_intent_metrics_sig`] accepts) and the exact tenant-binding string
/// the fabric bound the signature to. Injected at runtime from env/config —
/// NEVER hardcoded, NEVER fetched live in this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FabricAttestConfig {
    /// The fabric ed25519 verifying key, base64 (RFC 4648 §4, padded).
    pub fabric_pubkey_b64: String,
    /// The tenant string the `intent_metrics_sig` pre-image is bound to (anti-replay
    /// across tenants — part of the signed binding, must match exactly).
    pub tenant: String,
}

/// Env var holding the fabric ed25519 verifying key (standard-base64).
pub const ENV_FABRIC_PUBKEY: &str = "HUGIT_FABRIC_PUBKEY_B64";
/// Env var holding the tenant-binding string the sig covers.
pub const ENV_FABRIC_TENANT: &str = "HUGIT_FABRIC_TENANT";

impl FabricAttestConfig {
    /// Load the config from the environment. Returns `None` (⇒ the verdict is
    /// honestly [`UnattestedReason::NoPubkey`]) when EITHER piece is unset or blank
    /// — fail-closed, never a fabricated key. A present-but-blank value is treated
    /// as absent.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let pubkey = non_blank(std::env::var(ENV_FABRIC_PUBKEY).ok())?;
        let tenant = non_blank(std::env::var(ENV_FABRIC_TENANT).ok())?;
        Some(Self {
            fabric_pubkey_b64: pubkey,
            tenant,
        })
    }

    /// `true` iff the pubkey is present + non-blank (a blank key can never attest —
    /// it verifies nothing). Blank ⇒ the verdict is `NoPubkey`, never `SigInvalid`.
    fn has_pubkey(&self) -> bool {
        !self.fabric_pubkey_b64.trim().is_empty()
    }
}

/// `Some(trimmed-non-empty)` iff `v` is present and not all-whitespace.
fn non_blank(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.trim().is_empty())
}

/// Why a cost figure is NOT attested. Each variant is returned DISTINCTLY so the
/// caller can react precisely (esp. [`SigInvalid`](Self::SigInvalid), a hard fabric
/// signal, vs the benign [`EmissionOff`](Self::EmissionOff) default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnattestedReason {
    /// No fabric pubkey/tenant configured — nothing to verify against (fail-closed).
    NoPubkey,
    /// The close carried no `intent_metrics_sig` — the fabric's emission is off
    /// (`FABRIC_EMIT_INTENT_METRICS_SIG` unset, the default). The cost is
    /// fabric-recorded but UNATTESTED — the benign, expected case today.
    EmissionOff,
    /// A sig WAS present but did NOT verify — a HARD signal: a bad/hostile fabric or
    /// a MITM. The caller MUST refuse to claim the cost (never silently downgrade).
    SigInvalid,
}

impl core::fmt::Display for UnattestedReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            UnattestedReason::NoPubkey => "no pubkey",
            UnattestedReason::EmissionOff => "emission off",
            UnattestedReason::SigInvalid => "sig invalid",
        })
    }
}

/// The cost-integrity verdict over a [`DispatchOutcome`]'s finalized cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostAttestation {
    /// The finalized cost is ATTESTED: a present `intent_metrics_sig` verified under
    /// the configured fabric pubkey for this lease + tenant + metrics.
    Attested,
    /// The cost is NOT attested — carries the precise reason (never a silent pass).
    Unattested(UnattestedReason),
}

impl CostAttestation {
    /// `true` ONLY for [`Attested`](Self::Attested). Convenience for a caller that
    /// gates a `✓ cas:` marker / an attested-cost render on a verified signature.
    #[must_use]
    pub fn is_attested(&self) -> bool {
        matches!(self, CostAttestation::Attested)
    }
}

/// Compute the fail-closed cost-attestation verdict for an off-box dispatch.
///
/// Verifies the outcome's `intent_metrics_sig` against the configured fabric pubkey,
/// bound to the outcome's `lease_id` + the configured `tenant` + the finalized
/// metrics (reconstructed byte-exactly into the [`RunnerJobMetrics`] the fabric
/// signed). Renders/emits NOTHING — returns only the typed verdict.
///
/// Precedence (each fails closed, never up-grades):
///   1. `config == None` (or a blank pubkey) ⇒ [`UnattestedReason::NoPubkey`];
///   2. `outcome.intent_metrics_sig == None` ⇒ [`UnattestedReason::EmissionOff`];
///   3. a present sig that verifies ⇒ [`CostAttestation::Attested`];
///   4. a present sig that does NOT verify ⇒ [`UnattestedReason::SigInvalid`].
#[must_use]
pub fn cost_attestation_verdict(
    config: Option<&FabricAttestConfig>,
    outcome: &DispatchOutcome,
) -> CostAttestation {
    // (1) No usable config → cannot verify. Never a fabricated Attested.
    let Some(cfg) = config.filter(|c| c.has_pubkey()) else {
        return CostAttestation::Unattested(UnattestedReason::NoPubkey);
    };
    // (2) No sig on the close → the fabric's emission is off (the benign default).
    let Some(sig) = outcome.intent_metrics_sig.as_deref() else {
        return CostAttestation::Unattested(UnattestedReason::EmissionOff);
    };
    // Reconstruct the EXACT `RunnerJobMetrics` the fabric signed. The reverse
    // mapping is field-identical + lossless, so the `intent_metrics_preimage` bytes
    // are byte-for-byte what the fabric bound.
    let signed: RunnerJobMetrics = outcome.metrics.clone().into();
    // (3)/(4) fail-closed verify: Attested ONLY on a valid sig; a present-but-invalid
    // sig is a HARD, distinct signal.
    if verify_intent_metrics_sig(
        &cfg.fabric_pubkey_b64,
        &outcome.lease_id,
        &cfg.tenant,
        &signed,
        sig,
    ) {
        CostAttestation::Attested
    } else {
        CostAttestation::Unattested(UnattestedReason::SigInvalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::metrics::{RunnerJobMetrics, RunnerTokenCounts, RunnerToolCount};
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    use ed25519_dalek::{Signer, SigningKey};
    use hugit_contracts::CheckResult;

    const LEASE: &str = "lease-cost-1";
    const TENANT: &str = "d863fafb";

    /// A distinctive §13.1 metrics payload — the fabric's finalized close figure.
    fn sample_runner_metrics() -> RunnerJobMetrics {
        RunnerJobMetrics {
            tokens: RunnerTokenCounts {
                input: 100,
                output: 50,
                cache_read: 10,
                cache_write: 5,
                total: 165,
            },
            wall_ms: 4200,
            active_ms: 3100,
            tool_calls: 3,
            tool_breakdown: vec![
                RunnerToolCount {
                    tool: "Bash".into(),
                    count: 2,
                },
                RunnerToolCount {
                    tool: "Edit".into(),
                    count: 1,
                },
            ],
            model_turns: 2,
            cost_usd_micros: 20_340_000,
        }
    }

    /// Sign a `RunnerJobMetrics` exactly as the fabric would (the shared
    /// single-sourced pre-image), returning the base64 detached ed25519 sig.
    fn sign(signing: &SigningKey, lease: &str, tenant: &str, m: &RunnerJobMetrics) -> String {
        let breakdown: Vec<(String, u64)> = m
            .tool_breakdown
            .iter()
            .map(|t| (t.tool.clone(), t.count))
            .collect();
        let preimage = hugit_refstore::intent_metrics_preimage(
            lease,
            tenant,
            m.tokens.input,
            m.tokens.output,
            m.tokens.cache_read,
            m.tokens.cache_write,
            m.tokens.total,
            m.wall_ms,
            m.active_ms,
            m.tool_calls,
            &breakdown,
            m.model_turns,
            m.cost_usd_micros,
        );
        B64.encode(signing.sign(&preimage).to_bytes())
    }

    /// Build a `DispatchOutcome` carrying the given metrics + sig (the shape
    /// `dispatch_attest_offbox` produces). The `result` is a minimal honest stub —
    /// the verdict ignores it.
    fn outcome(m: &RunnerJobMetrics, sig: Option<String>) -> DispatchOutcome {
        DispatchOutcome {
            result: CheckResult {
                memo_key: "f".repeat(64),
                tree_hash: "1".repeat(64),
                def_digest: "2".repeat(64),
                toolchain_digest: "3".repeat(64),
                exit: 0,
                artifacts: Vec::new(),
                stdout_ref: String::new(),
                stderr_ref: String::new(),
                duration_ms: m.wall_ms,
                runner_ref: format!("offbox:{LEASE}"),
                produced_at: 0,
            },
            metrics: m.clone().into_intent_metrics(),
            lease_id: LEASE.to_string(),
            intent_metrics_sig: sig,
            fabric_key_id: Some("0011223344556677".to_string()),
        }
    }

    fn config(pubkey_b64: &str) -> FabricAttestConfig {
        FabricAttestConfig {
            fabric_pubkey_b64: pubkey_b64.to_string(),
            tenant: TENANT.to_string(),
        }
    }

    /// A close carrying a VALID sig → verdict `Attested`.
    #[test]
    fn valid_sig_verdict_is_attested() {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey = B64.encode(signing.verifying_key().to_bytes());
        let m = sample_runner_metrics();
        let sig = sign(&signing, LEASE, TENANT, &m);
        let out = outcome(&m, Some(sig));

        let cfg = config(&pubkey);
        assert_eq!(
            cost_attestation_verdict(Some(&cfg), &out),
            CostAttestation::Attested
        );
        assert!(cost_attestation_verdict(Some(&cfg), &out).is_attested());
    }

    /// An ABSENT sig (fabric emission off) → `Unattested(EmissionOff)`, even with a
    /// valid pubkey configured — the honest default today.
    #[test]
    fn absent_sig_verdict_is_unattested_emission_off() {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey = B64.encode(signing.verifying_key().to_bytes());
        let m = sample_runner_metrics();
        let out = outcome(&m, None); // no sig on the close

        assert_eq!(
            cost_attestation_verdict(Some(&config(&pubkey)), &out),
            CostAttestation::Unattested(UnattestedReason::EmissionOff)
        );
    }

    /// A PRESENT-but-TAMPERED sig → `Unattested(SigInvalid)`, returned DISTINCTLY
    /// from `EmissionOff` (a bad fabric is a hard signal, not a benign absence).
    #[test]
    fn present_but_invalid_sig_verdict_is_unattested_sig_invalid() {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey = B64.encode(signing.verifying_key().to_bytes());
        let m = sample_runner_metrics();
        let sig = sign(&signing, LEASE, TENANT, &m);

        // (a) A cost tamper: the outcome's finalized cost differs from what was
        // signed → the reconstructed pre-image no longer matches → invalid.
        let mut tampered = m.clone();
        tampered.cost_usd_micros += 1;
        let out_cost = outcome(&tampered, Some(sig.clone()));
        assert_eq!(
            cost_attestation_verdict(Some(&config(&pubkey)), &out_cost),
            CostAttestation::Unattested(UnattestedReason::SigInvalid),
            "a cost tamper must be SigInvalid (distinct from EmissionOff)"
        );

        // (b) A garbage sig string over the genuine metrics → also SigInvalid
        // (fail-closed on a malformed/non-verifying sig, never a panic).
        let out_bad = outcome(&m, Some("not-a-real-sig".to_string()));
        assert_eq!(
            cost_attestation_verdict(Some(&config(&pubkey)), &out_bad),
            CostAttestation::Unattested(UnattestedReason::SigInvalid)
        );

        // (c) Wrong fabric key → SigInvalid (a genuine sig under the wrong key).
        let other = B64.encode(
            SigningKey::from_bytes(&[9u8; 32])
                .verifying_key()
                .to_bytes(),
        );
        let out_ok = outcome(&m, Some(sig));
        assert_eq!(
            cost_attestation_verdict(Some(&config(&other)), &out_ok),
            CostAttestation::Unattested(UnattestedReason::SigInvalid)
        );
    }

    /// NO pubkey config (or a blank pubkey) → `Unattested(NoPubkey)` — never a
    /// fabricated Attested, even when the outcome carries a sig.
    #[test]
    fn no_pubkey_config_verdict_is_unattested_no_pubkey() {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let m = sample_runner_metrics();
        let sig = sign(&signing, LEASE, TENANT, &m);
        let out = outcome(&m, Some(sig));

        // (a) config absent entirely.
        assert_eq!(
            cost_attestation_verdict(None, &out),
            CostAttestation::Unattested(UnattestedReason::NoPubkey)
        );
        // (b) config present but the pubkey is blank → still NoPubkey (never
        // SigInvalid — a blank key can attest nothing).
        assert_eq!(
            cost_attestation_verdict(Some(&config("   ")), &out),
            CostAttestation::Unattested(UnattestedReason::NoPubkey)
        );
    }

    /// Anti-replay: a valid sig under the WRONG tenant binding fails (SigInvalid) —
    /// the tenant is part of the signed pre-image, supplied here by config.
    #[test]
    fn wrong_tenant_binding_is_sig_invalid() {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey = B64.encode(signing.verifying_key().to_bytes());
        let m = sample_runner_metrics();
        let sig = sign(&signing, LEASE, TENANT, &m);
        let out = outcome(&m, Some(sig));

        let wrong = FabricAttestConfig {
            fabric_pubkey_b64: pubkey,
            tenant: "other-tenant".to_string(),
        };
        assert_eq!(
            cost_attestation_verdict(Some(&wrong), &out),
            CostAttestation::Unattested(UnattestedReason::SigInvalid)
        );
    }

    /// The reasons render to their canonical strings (stable for logging).
    #[test]
    fn unattested_reasons_render_canonically() {
        assert_eq!(UnattestedReason::NoPubkey.to_string(), "no pubkey");
        assert_eq!(UnattestedReason::EmissionOff.to_string(), "emission off");
        assert_eq!(UnattestedReason::SigInvalid.to_string(), "sig invalid");
    }
}

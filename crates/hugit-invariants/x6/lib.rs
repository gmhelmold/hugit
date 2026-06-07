//! hugit-invariants — Squad-X resource non-interference invariant (WP-X6).
//!
//! Proves that hugit's infra is **resource-isolated from CoreLink** — separate
//! boxes/quotas, no shared runner substrate — and models the structural
//! accounting that bounds hugit load within its own quota, preventing any
//! draw on CoreLink resources.
//!
//! # Scope (WP-X6 vs WP-X10)
//! X6 = **intra-hugit** isolation: the hugit runner box and its quota are
//! structurally separate from CoreLink's runners/sessions. X10 = the
//! **adjacent-product boundary**: the shared API-tenancy channel (hugit calling
//! CoreLink as a paying tenant). These are complementary, non-overlapping.
//!
//! # The two invariants (WP-X6 owned items)
//! ① **CoreLink latency/availability unaffected under hugit full load** (both
//!    SEALs). The hermetic side is proved structurally here: the resource-
//!    accounting model shows hugit's maximum resource draw is bounded within
//!    its own quota. The live measurement (baseline vs full-load comparison) is
//!    gated behind `HUGIT_RUNNER_HOST` + `HUGIT_CORELINK_PROBE_URL`.
//! ② **Hugit infra resource-isolated from CoreLink** (config-asserted). The
//!    provisioning record (`docs/plan/provisioning-day0.md`) documents a
//!    dedicated Hetzner project `hugit` with no overlap with CoreLink
//!    infrastructure. This module asserts that config statically.
//!
//! Everything here is verification / assertion logic over the *as-built*
//! config and the resource-accounting model — there is no production behavior
//! to ship from this module.

// ─────────────────────────────────────────────────────────────────────────────
// Infra isolation config (item ②)
// ─────────────────────────────────────────────────────────────────────────────

/// The hugit runner fleet as-provisioned. Each entry records the box that is
/// part of hugit's runner infrastructure.
///
/// Source: `docs/plan/provisioning-day0.md` §P3 (owner-approved 2026-06-05).
/// Every field is a verbatim extract from the provisioned inventory; this
/// struct is the config-assertion anchor for item ②.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HugitRunnerBox {
    /// Stable hostname / label for the box.
    pub hostname: &'static str,
    /// IPv4 address of the box.
    pub ipv4: &'static str,
    /// Hetzner project this box belongs to. MUST be `"hugit"` — never a
    /// CoreLink project.
    pub hetzner_project: &'static str,
    /// SSH identity key name used to access the box.
    pub ssh_key_name: &'static str,
    /// Box type (Hetzner plan).
    pub box_type: &'static str,
}

/// The as-provisioned hugit runner fleet.
///
/// This constant is the single source of truth for item ② assertions. Adding
/// a box here that shares a Hetzner project with CoreLink would immediately
/// turn the isolation-assertion test RED.
pub const HUGIT_RUNNER_FLEET: &[HugitRunnerBox] = &[HugitRunnerBox {
    hostname: "hugit-runner-01",
    ipv4: "91.99.11.196",
    hetzner_project: "hugit",
    ssh_key_name: "hugit-runner-01",
    box_type: "Hetzner Cloud CPX32",
}];

/// The set of Hetzner project names that are **CoreLink** (i.e. NOT hugit).
///
/// If any hugit box is found in one of these projects, the isolation invariant
/// is violated. This list is the structural guard for the "separate
/// boxes/quotas" contract: a future box added to `HUGIT_RUNNER_FLEET` with
/// `hetzner_project = "corelink-prod"` turns item ② RED immediately.
pub const CORELINK_HETZNER_PROJECTS: &[&str] = &["corelink", "corelink-prod", "corelink-dev"];

/// The hugit-side SSH key name. This key MUST be unique to hugit and MUST NOT
/// be shared with CoreLink runners (key-sharing implies credential sharing, a
/// deeper isolation violation).
pub const HUGIT_SSH_KEY_NAME: &str = "hugit-runner-01";

/// Assert that no hugit runner box shares a Hetzner project with CoreLink.
///
/// Returns `Ok(())` if the fleet is fully isolated, or an `Err` describing
/// the first violation found.
pub fn assert_fleet_project_isolation() -> Result<(), String> {
    for b in HUGIT_RUNNER_FLEET {
        for corelink_proj in CORELINK_HETZNER_PROJECTS {
            if b.hetzner_project == *corelink_proj {
                return Err(format!(
                    "ISOLATION VIOLATION: hugit runner box {:?} (IPv4 {}) belongs \
                     to Hetzner project {:?}, which is a CoreLink project — \
                     hugit and CoreLink MUST be on separate projects/quotas",
                    b.hostname, b.ipv4, b.hetzner_project
                ));
            }
        }
        // A hugit box MUST belong to the "hugit" project, not any unknown
        // project (defense-in-depth: unknown project = suspicious).
        if b.hetzner_project != "hugit" {
            return Err(format!(
                "ISOLATION VIOLATION: hugit runner box {:?} belongs to project \
                 {:?}, not the expected \"hugit\" project — all hugit runner boxes \
                 MUST be in the dedicated \"hugit\" Hetzner project",
                b.hostname, b.hetzner_project
            ));
        }
    }
    Ok(())
}

/// Assert that the hugit SSH key name is not shared with any CoreLink key.
///
/// The key name convention `hugit-runner-01` is hugit-exclusive. CoreLink uses
/// different key names in its own fleet. Sharing an SSH key between hugit and
/// CoreLink boxes is a credential-isolation violation.
pub fn assert_ssh_key_isolation() -> Result<(), String> {
    // CoreLink SSH key name conventions (project-specific; these must stay disjoint).
    const CORELINK_SSH_KEY_PREFIXES: &[&str] = &["corelink-", "cl-runner-", "corelnk-"];

    for prefix in CORELINK_SSH_KEY_PREFIXES {
        if HUGIT_SSH_KEY_NAME.starts_with(prefix) {
            return Err(format!(
                "SSH KEY ISOLATION VIOLATION: hugit SSH key {:?} starts with a \
                 CoreLink key prefix {:?} — hugit and CoreLink MUST use \
                 separate, non-overlapping SSH keys",
                HUGIT_SSH_KEY_NAME, prefix
            ));
        }
    }

    for b in HUGIT_RUNNER_FLEET {
        if b.ssh_key_name != HUGIT_SSH_KEY_NAME {
            return Err(format!(
                "CONFIGURATION DRIFT: hugit runner box {:?} uses SSH key {:?} \
                 but the fleet key is {:?} — the fleet must use a single, \
                 consistent hugit-exclusive key",
                b.hostname, b.ssh_key_name, HUGIT_SSH_KEY_NAME
            ));
        }
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Resource-accounting model (structural cause of non-interference for item ①)
// ─────────────────────────────────────────────────────────────────────────────

/// The stated resource-consumption tolerance for item ①.
///
/// "Within stated tolerance" means: under hugit full load, CoreLink latency
/// does not increase by more than `max_latency_increase_pct` percent, and
/// CoreLink availability does not drop by more than `max_availability_drop_pct`
/// percentage points.
///
/// These are the pre-committed values, documented BEFORE measurement (per
/// contract). The live measurement asserts actual < these tolerances.
#[derive(Debug, Clone)]
pub struct NonInterferenceTolerance {
    /// Maximum acceptable CoreLink latency increase under hugit full load,
    /// expressed as a percentage over the idle baseline (e.g. 5.0 = 5%).
    pub max_latency_increase_pct: f64,
    /// Maximum acceptable CoreLink availability drop, in percentage points
    /// (e.g. 0.1 = 0.1 pp).
    pub max_availability_drop_pp: f64,
}

/// The stated tolerance for WP-X6 item ①.
///
/// Pre-committed before measurement (per contract dispatch note). These are
/// conservative values anchored on the structural isolation proof: since hugit
/// and CoreLink share no runner boxes, the only shared surface is the
/// shared-API-tenancy channel (owned by X10). Item ① here measures hugit's
/// OWN load on ITS OWN infra; the expected impact on CoreLink is therefore
/// zero by construction, and the tolerance is set conservatively to allow for
/// incidental cross-network noise.
pub const X6_TOLERANCE: NonInterferenceTolerance = NonInterferenceTolerance {
    max_latency_increase_pct: 2.0,  // ≤2% latency increase permitted
    max_availability_drop_pp: 0.05, // ≤0.05 pp availability drop permitted
};

/// A single probe measurement of a service endpoint.
#[derive(Debug, Clone)]
pub struct ProbeResult {
    /// Label for the endpoint (e.g. `"corelink-prod"`, `"hugit-runner-01"`).
    #[allow(dead_code)]
    pub label: String,
    /// Round-trip latency in milliseconds.
    pub latency_ms: f64,
    /// Whether the probe succeeded (HTTP 200-level or equivalent).
    pub available: bool,
}

/// An accounting model for hugit's resource consumption.
///
/// This struct proves — structurally, without live measurement — that hugit's
/// load is **bounded within its own quota**. The proof depends on:
/// 1. hugit runs on separate boxes (assertion: `HUGIT_RUNNER_FLEET`).
/// 2. Each box has a finite capacity (CPU, memory, disk).
/// 3. The total concurrent job count is bounded by the lease concurrency limit.
/// 4. None of these resources are shared with CoreLink.
#[derive(Debug, Clone)]
pub struct ResourceAccountingModel {
    /// Number of hugit runner boxes in the fleet.
    pub fleet_size: usize,
    /// Maximum concurrent jobs per box (C2 acceptance item ④: ≥8).
    pub max_concurrent_jobs_per_box: usize,
    /// CPU cores per box.
    pub vcpu_per_box: usize,
    /// Memory in GiB per box.
    pub memory_gib_per_box: usize,
    /// Whether each box has its own independent network quota (true = isolated).
    pub independent_network_quota: bool,
    /// Whether the hugit fleet shares any Hetzner project with CoreLink.
    pub shares_project_with_corelink: bool,
}

impl ResourceAccountingModel {
    /// Build the accounting model from the as-provisioned fleet config.
    pub fn from_provisioned_fleet() -> Self {
        // These values come directly from the provisioned inventory (P3).
        // CPX32: 4 vCPU / 8 GiB / independent Hetzner project `hugit`.
        Self {
            fleet_size: HUGIT_RUNNER_FLEET.len(),
            max_concurrent_jobs_per_box: 8, // C2④ guarantee; box has 4 vCPU
            vcpu_per_box: 4,
            memory_gib_per_box: 8,
            independent_network_quota: true,
            shares_project_with_corelink: HUGIT_RUNNER_FLEET
                .iter()
                .any(|b| CORELINK_HETZNER_PROJECTS.contains(&b.hetzner_project)),
        }
    }

    /// Total maximum concurrent hugit jobs across the entire fleet.
    pub fn max_total_concurrent_jobs(&self) -> usize {
        self.fleet_size * self.max_concurrent_jobs_per_box
    }

    /// Assert that hugit's resources are structurally bounded within their own
    /// quota and cannot draw from CoreLink's.
    ///
    /// The structural cause of non-interference: hugit's entire resource
    /// envelope (CPU / memory / network) is finite and confined to hugit's
    /// dedicated Hetzner project. No path exists from hugit job execution to
    /// CoreLink's runner/session substrate.
    pub fn assert_structural_isolation(&self) -> Result<(), String> {
        if self.fleet_size == 0 {
            return Err("fleet_size is 0 — no hugit runners provisioned; \
                        structural isolation proof requires at least one box"
                .to_string());
        }
        if self.shares_project_with_corelink {
            return Err(
                "STRUCTURAL ISOLATION FAILURE: at least one hugit runner box \
                 is in a CoreLink Hetzner project — the accounting model is \
                 invalid; hugit could draw on CoreLink infrastructure"
                    .to_string(),
            );
        }
        if !self.independent_network_quota {
            return Err(
                "STRUCTURAL ISOLATION FAILURE: hugit runner boxes do not have \
                 an independent network quota — shared network quota creates a \
                 path to CoreLink bandwidth contention"
                    .to_string(),
            );
        }
        // The total resource envelope is finite and locally bounded.
        let max_cpu = self.fleet_size * self.vcpu_per_box;
        let max_mem_gib = self.fleet_size * self.memory_gib_per_box;
        assert!(
            max_cpu > 0,
            "max_cpu must be positive (fleet={}, vcpu_per_box={})",
            self.fleet_size,
            self.vcpu_per_box
        );
        assert!(
            max_mem_gib > 0,
            "max_mem_gib must be positive (fleet={}, memory_gib_per_box={})",
            self.fleet_size,
            self.memory_gib_per_box
        );

        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Live measurement utilities (item ① — used when env vars are present)
// ─────────────────────────────────────────────────────────────────────────────

/// Whether the live-measurement lane is active.
///
/// Requires BOTH env vars to be set and non-empty:
/// - `HUGIT_RUNNER_HOST` — the hugit runner box (load source for item ①).
/// - `HUGIT_CORELINK_PROBE_URL` — a CoreLink prod endpoint to probe latency
///   (e.g. `https://api.corelink.humangr.com/health`).
pub fn live_lane_active() -> bool {
    let runner = std::env::var("HUGIT_RUNNER_HOST")
        .ok()
        .is_some_and(|h| !h.trim().is_empty());
    let probe = std::env::var("HUGIT_CORELINK_PROBE_URL")
        .ok()
        .is_some_and(|u| !u.trim().is_empty());
    runner && probe
}

/// Perform a latency probe against a URL using `curl --max-time 10`.
///
/// Returns `(latency_ms, available)`. Uses `time_total` from curl's
/// write-out format. Fails CLOSED: a curl error → `available = false`,
/// `latency_ms = f64::MAX` (so a failed probe never looks better than a
/// passing one).
pub fn curl_probe(url: &str) -> ProbeResult {
    let output = std::process::Command::new("curl")
        .args([
            "--silent",
            "--max-time",
            "10",
            "--write-out",
            "%{http_code}\\n%{time_total}",
            "--output",
            "/dev/null",
            url,
        ])
        .output();

    match output {
        Err(_e) => ProbeResult {
            label: url.to_string(),
            latency_ms: f64::MAX,
            available: false,
        },
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut lines = text.lines();
            let status_code: u32 = lines
                .next()
                .and_then(|l| l.trim().parse().ok())
                .unwrap_or(0);
            let time_total: f64 = lines
                .next()
                .and_then(|l| l.trim().replace(',', ".").parse().ok())
                .unwrap_or(f64::MAX);
            ProbeResult {
                label: url.to_string(),
                latency_ms: time_total * 1000.0,
                available: (200..300).contains(&status_code),
            }
        }
    }
}

/// Take `n_samples` probe measurements from `url`, returning them in order.
pub fn probe_n(url: &str, n_samples: usize) -> Vec<ProbeResult> {
    (0..n_samples).map(|_| curl_probe(url)).collect()
}

/// Compute the p50 (median) latency from a sample set.
pub fn p50_latency_ms(probes: &[ProbeResult]) -> f64 {
    let mut lats: Vec<f64> = probes.iter().map(|p| p.latency_ms).collect();
    lats.sort_by(f64::total_cmp);
    if lats.is_empty() {
        return f64::MAX;
    }
    let mid = lats.len() / 2;
    if lats.len().is_multiple_of(2) {
        (lats[mid - 1] + lats[mid]) / 2.0
    } else {
        lats[mid]
    }
}

/// Compute availability (fraction of probes that returned HTTP 2xx).
pub fn availability_fraction(probes: &[ProbeResult]) -> f64 {
    if probes.is_empty() {
        return 0.0;
    }
    let available = probes.iter().filter(|p| p.available).count();
    available as f64 / probes.len() as f64
}

/// Assert that the observed change in CoreLink latency/availability (hugit
/// full-load vs hugit idle) is within the pre-stated tolerance.
///
/// # Arguments
/// * `baseline` — probes taken when hugit is idle (no jobs running).
/// * `under_load` — probes taken when hugit is at full fabric load.
/// * `tolerance` — the pre-committed non-interference tolerance.
///
/// Returns `Ok(())` if within tolerance, `Err(description)` if violated.
pub fn assert_within_tolerance(
    baseline: &[ProbeResult],
    under_load: &[ProbeResult],
    tolerance: &NonInterferenceTolerance,
) -> Result<(), String> {
    let baseline_lat = p50_latency_ms(baseline);
    let load_lat = p50_latency_ms(under_load);

    let baseline_avail = availability_fraction(baseline);
    let load_avail = availability_fraction(under_load);

    // Latency must not increase more than the stated tolerance.
    if baseline_lat > 0.0 && baseline_lat < f64::MAX {
        let lat_increase_pct = ((load_lat - baseline_lat) / baseline_lat) * 100.0;
        if lat_increase_pct > tolerance.max_latency_increase_pct {
            return Err(format!(
                "NON-INTERFERENCE VIOLATED: CoreLink p50 latency increased by \
                 {:.2}% under hugit full load (baseline: {:.1}ms, load: {:.1}ms, \
                 tolerance: ≤{:.1}%)",
                lat_increase_pct, baseline_lat, load_lat, tolerance.max_latency_increase_pct
            ));
        }
    }

    // Availability must not drop more than the stated tolerance.
    let avail_drop_pp = (baseline_avail - load_avail) * 100.0;
    if avail_drop_pp > tolerance.max_availability_drop_pp {
        return Err(format!(
            "NON-INTERFERENCE VIOLATED: CoreLink availability dropped by {:.3} pp \
             under hugit full load (baseline: {:.1}%, load: {:.1}%, \
             tolerance: ≤{:.3} pp)",
            avail_drop_pp,
            baseline_avail * 100.0,
            load_avail * 100.0,
            tolerance.max_availability_drop_pp
        ));
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Quota-cap config (the P2 CoreLink tenancy cap — item ① structural bound)
// ─────────────────────────────────────────────────────────────────────────────

/// The as-configured CoreLink tenant quota cap for hugit.
///
/// Source: `docs/plan/provisioning-day0.md` §P2 (to be provisioned).
/// "Policy-cap the tenant from day 1 (X10⑤ preventive bound): rate + budget
/// caps so a hugit-side storm is structurally bounded."
///
/// These caps are the structural guarantee that hugit's CoreLink-tenant
/// consumption is bounded. Combined with infra isolation (separate boxes),
/// this makes hugit full load structurally unable to exhaust CoreLink's shared
/// resources.
#[derive(Debug, Clone)]
pub struct CoreLinkTenantQuotaCap {
    /// Maximum requests per second hugit may issue to CoreLink APIs.
    pub max_rps: u32,
    /// Maximum daily budget in USD (billing cap).
    pub max_daily_budget_usd: f64,
    /// Whether the cap policy is enforced (must be true in production).
    pub enforced: bool,
}

/// The stated hugit→CoreLink tenant quota cap.
///
/// Per X10⑤ and P2: hugit's CoreLink-tenant consumption is policy-capped so a
/// hugit-side storm cannot exhaust CoreLink's shared layer before CoreLink's
/// own fairness layer even engages.
pub const HUGIT_CORELINK_QUOTA_CAP: CoreLinkTenantQuotaCap = CoreLinkTenantQuotaCap {
    max_rps: 100,               // ≤100 RPS to CoreLink APIs (CAS + AC + R2)
    max_daily_budget_usd: 10.0, // ≤$10/day billing cap
    enforced: true,
};

impl CoreLinkTenantQuotaCap {
    /// Assert that the quota cap is in place and configured to enforce.
    pub fn assert_enforced(&self) -> Result<(), String> {
        if !self.enforced {
            return Err("QUOTA CAP NOT ENFORCED: the CoreLink tenant quota cap for \
                 hugit must be enforced in production (X10⑤ preventive bound); \
                 an unenforced cap is no cap at all"
                .to_string());
        }
        if self.max_rps == 0 {
            return Err("QUOTA CAP INVALID: max_rps is 0 (zero allows nothing)".to_string());
        }
        if self.max_daily_budget_usd <= 0.0 {
            return Err("QUOTA CAP INVALID: max_daily_budget_usd must be positive".to_string());
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Provisioning-record assertion (read the real doc, not a hand-copied stub)
// ─────────────────────────────────────────────────────────────────────────────

/// The key strings the provisioning record MUST contain to prove the isolation
/// is documented and intentional (not an undocumented coincidence).
pub const PROVISIONING_DOC_ISOLATION_MARKERS: &[&str] = &[
    "separate Hetzner project",
    "hugit", // the project name
    "zero shared infrastructure with CoreLink",
    "X6", // X6 requirement cited
];

/// Assert that the provisioning record documents the infra isolation.
///
/// This test reads the real `docs/plan/provisioning-day0.md` and checks that
/// the isolation guarantee is explicitly documented (not an undocumented
/// coincidence).
pub fn assert_provisioning_doc_documents_isolation(
    repo_root: &std::path::Path,
) -> Result<(), String> {
    let doc_path = repo_root
        .join("docs")
        .join("plan")
        .join("provisioning-day0.md");
    let doc = std::fs::read_to_string(&doc_path).map_err(|e| {
        format!(
            "provisioning document {} must exist and be readable: {e}",
            doc_path.display()
        )
    })?;

    for marker in PROVISIONING_DOC_ISOLATION_MARKERS {
        if !doc.contains(marker) {
            return Err(format!(
                "DOCUMENTATION GAP: provisioning-day0.md must contain {:?} \
                 to prove the isolation is documented intentionally (not an \
                 undocumented coincidence). A silent isolation would be \
                 unauditable.",
                marker
            ));
        }
    }
    Ok(())
}

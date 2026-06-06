//! Secrets broker (WP-C5b) — CoreLink's write-only secret model, generalized.
//!
//! **Credentials never enter the runner.** A claim-fenced workspace that needs
//! a privileged operation does not receive a credential; it sends the broker a
//! [`BrokerRequest`] describing *what* to do, and the broker — which alone
//! holds the secret — performs the operation on the workspace's behalf and
//! returns only the **result** (whitepaper §9 lock 2). The runner never sees,
//! and cannot derive, the raw credential.
//!
//! Three guarantees this module enforces (the C5b owned acceptance items):
//! - **② zero secret material in the job:** the secret lives only inside the
//!   broker's transient privileged step on the box; it is never written into
//!   the job container's env, argv, or disk. The positive path (⑥) proves this
//!   with an after-the-fact scan ([`scan_credential_absent`]).
//! - **③ audited with the principal chain:** every broker call records an
//!   [`AuditRecord`] carrying the full `principal_chain` from the
//!   [`RunnerLease`]. The audit log never contains the secret.
//! - **④ fail CLOSED:** when the secret store is unreachable the operation
//!   returns [`BrokerError::BrokerDown`] and the workspace operation **fails** —
//!   it never degrades to a credential-on-runner fallback (§9 lock 5).
//!
//! The broker rides on C5a's frozen fence: it never re-implements
//! materialization or the ENOENT enforcement, and holds no fallback path that
//! would place a credential inside the fence.

use std::collections::BTreeMap;
use std::fmt;

use hugit_contracts::RunnerLease;
use hugit_runner::isolation::RunningContainer;
use hugit_runner::lease::BoxExec;

/// An opaque handle to a secret the broker holds. It carries the secret's
/// **name only** — never the material — so it is safe to log and to embed in a
/// [`BrokerRequest`] that originates inside the runner.
///
/// The runner constructs a `SecretRef` by name; resolving it to material is the
/// broker's privilege alone (via a [`SecretStore`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRef {
    /// Logical name of the secret (e.g. `"deploy-signing-key"`). Never the
    /// material.
    pub name: String,
}

impl SecretRef {
    /// Reference a secret by name.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// The privileged operation a workspace asks the broker to perform on its
/// behalf. The workspace supplies only **public** inputs; the credential is
/// supplied by the broker, never by the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerOp {
    /// Authenticate a workspace-supplied message by signing it with the
    /// broker-held secret keyed by `secret`, returning the signature.
    ///
    /// This models the entire class of "the credential authenticates an
    /// operation" (sign a release, mint a token, authenticate a push): the
    /// runner provides the public `message`, the broker provides the secret,
    /// and only the resulting signature crosses back — never the key.
    Sign {
        /// The secret to sign with (by reference, never by value).
        secret: SecretRef,
        /// Public message the workspace wants authenticated.
        message: Vec<u8>,
    },
}

impl BrokerOp {
    /// The `SecretRef` this op needs.
    #[must_use]
    pub fn secret_ref(&self) -> &SecretRef {
        match self {
            BrokerOp::Sign { secret, .. } => secret,
        }
    }

    /// A short, secret-free verb for the audit record.
    #[must_use]
    pub fn verb(&self) -> &'static str {
        match self {
            BrokerOp::Sign { .. } => "sign",
        }
    }
}

/// A request from a claim-fenced workspace to the broker.
///
/// It names a [`BrokerOp`] and carries the [`RunnerLease`] whose
/// `principal_chain` the broker audits. The request contains **no credential**
/// by construction — the only secret-shaped field anywhere is a [`SecretRef`]
/// (a name).
#[derive(Debug, Clone)]
pub struct BrokerRequest<'a> {
    /// The lease that authorizes the call; supplies the audited principal
    /// chain.
    pub lease: &'a RunnerLease,
    /// The privileged operation to perform.
    pub op: BrokerOp,
}

/// The result the broker returns to the workspace. Carries only the **output**
/// of the privileged op (e.g. a signature) — never the credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerResponse {
    /// The operation's public output (e.g. a hex signature).
    pub output: String,
    /// The audit record produced for this call (secret-free).
    pub audit: AuditRecord,
}

/// An audit record for one broker call — **③ principal-chain audit**.
///
/// Records the full principal chain from the lease, the lease id, the op verb,
/// and the secret *name* (never its material), plus the outcome. The record is
/// safe to persist and to print: it provably contains no credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    /// Lease id the call was made under.
    pub lease_id: String,
    /// The full ordered principal chain copied from the lease.
    pub principal_chain: Vec<String>,
    /// The op verb (e.g. `"sign"`).
    pub op: String,
    /// The *name* of the secret used (never the material).
    pub secret_name: String,
    /// Whether the broker completed the op.
    pub outcome: AuditOutcome,
}

/// Outcome of an audited broker call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOutcome {
    /// The privileged op completed and a result was returned.
    Completed,
    /// The op failed closed (e.g. the secret store was unreachable).
    FailedClosed,
}

impl AuditRecord {
    /// Render the record as a single secret-free log line.
    #[must_use]
    pub fn to_log_line(&self) -> String {
        format!(
            "broker.audit lease={} principals=[{}] op={} secret={} outcome={:?}",
            self.lease_id,
            self.principal_chain.join(">"),
            self.op,
            self.secret_name,
            self.outcome,
        )
    }
}

/// Errors the broker raises. The variants are **fail-closed by construction**:
/// there is no variant that signals "fell back to a credential on the runner".
#[derive(Debug)]
pub enum BrokerError {
    /// The secret store is unreachable — the operation **fails closed**. The
    /// caller's privileged op does not complete; no credential is placed on the
    /// runner as a fallback (§9 lock 5). This is the **④** guarantee.
    BrokerDown {
        /// The secret name that could not be resolved.
        secret_name: String,
        /// Why the store was considered down (transport/credential-store
        /// error). Never contains secret material.
        reason: String,
    },
    /// The requested secret is not known to the store. Also fail-closed.
    UnknownSecret {
        /// The secret name that was not found.
        secret_name: String,
    },
    /// A box command failed while performing the privileged op.
    Box(anyhow::Error),
}

impl fmt::Display for BrokerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BrokerError::BrokerDown {
                secret_name,
                reason,
            } => write!(
                f,
                "broker down resolving secret {secret_name:?}: {reason}; \
                 operation FAILS CLOSED (no credential-on-runner fallback)"
            ),
            BrokerError::UnknownSecret { secret_name } => {
                write!(f, "unknown secret {secret_name:?}; operation fails closed")
            }
            BrokerError::Box(e) => write!(f, "broker box command failed: {e}"),
        }
    }
}

impl std::error::Error for BrokerError {}

impl BrokerError {
    /// `true` iff this error is the broker-down fail-closed condition.
    #[must_use]
    pub fn is_broker_down(&self) -> bool {
        matches!(self, BrokerError::BrokerDown { .. })
    }
}

/// The privileged secret store. The broker is the **only** holder of secret
/// material; the runner never has a [`SecretStore`].
///
/// `resolve` returns the material for a named secret, or `Err` when the store
/// is unreachable / the name is unknown. A returned `Ok(None)` means "the store
/// is up but has no such secret" (→ [`BrokerError::UnknownSecret`]); a returned
/// `Err` means "the store is **down**" (→ [`BrokerError::BrokerDown`], the
/// fail-closed path).
pub trait SecretStore {
    /// Resolve `name` to its secret material.
    ///
    /// # Errors
    /// Returns `Err` when the store itself is unreachable (the fail-closed
    /// trigger). `Ok(None)` is an *up* store that lacks the secret.
    fn resolve(&self, name: &str) -> Result<Option<SecretMaterial>, StoreUnreachable>;
}

/// Secret material, held only inside the broker. It deliberately does **not**
/// implement [`Debug`]/[`Display`] in a way that prints the bytes, so it cannot
/// be accidentally logged.
pub struct SecretMaterial(Vec<u8>);

impl SecretMaterial {
    /// Wrap raw secret bytes (broker-side only).
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Borrow the bytes for the privileged op (broker-internal use only).
    #[must_use]
    fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SecretMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretMaterial(<redacted {} bytes>)", self.0.len())
    }
}

/// Signals the secret store is unreachable — the fail-closed trigger.
#[derive(Debug)]
pub struct StoreUnreachable(pub String);

impl fmt::Display for StoreUnreachable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "secret store unreachable: {}", self.0)
    }
}

impl std::error::Error for StoreUnreachable {}

/// The secrets broker: holds the [`SecretStore`], performs privileged ops on a
/// workspace's behalf, and audits every call with the principal chain.
pub struct Broker<S: SecretStore> {
    store: S,
}

impl<S: SecretStore> Broker<S> {
    /// Construct a broker over a secret store.
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Build the audit record for a request and outcome — **③**. Copies the
    /// principal chain from the lease; never contains secret material.
    fn audit(req: &BrokerRequest<'_>, outcome: AuditOutcome) -> AuditRecord {
        AuditRecord {
            lease_id: req.lease.lease_id.clone(),
            principal_chain: req.lease.principal_chain.clone(),
            op: req.op.verb().to_string(),
            secret_name: req.op.secret_ref().name.clone(),
            outcome,
        }
    }

    /// Execute a privileged operation **purely** (no box) — the broker resolves
    /// the secret and performs the op locally, returning only the result.
    ///
    /// This is the deterministic core used by both the pure unit tests and the
    /// box-backed positive path: the credential is consumed inside this call and
    /// never leaves it. On a down store it **fails closed**.
    ///
    /// # Errors
    /// - [`BrokerError::BrokerDown`] if the secret store is unreachable
    ///   (fail-closed; **④**).
    /// - [`BrokerError::UnknownSecret`] if the store lacks the secret.
    pub fn execute(&self, req: &BrokerRequest<'_>) -> Result<BrokerResponse, BrokerError> {
        let secret_name = req.op.secret_ref().name.clone();
        let material = match self.store.resolve(&secret_name) {
            Ok(Some(m)) => m,
            Ok(None) => return Err(BrokerError::UnknownSecret { secret_name }),
            Err(StoreUnreachable(reason)) => {
                // FAIL CLOSED: the op does not complete, and crucially we do not
                // hand any credential back to the runner. ④.
                return Err(BrokerError::BrokerDown {
                    secret_name,
                    reason,
                });
            }
        };

        let output = match &req.op {
            BrokerOp::Sign { message, .. } => sign_hmac_sha256(material.expose(), message),
        };

        Ok(BrokerResponse {
            output,
            audit: Self::audit(req, AuditOutcome::Completed),
        })
    }

    /// Execute the privileged op and **deliver only the result into the runner
    /// container** on the box — the credential never touches the container.
    ///
    /// This is the positive path (**⑥**): the broker resolves and uses the
    /// secret entirely on the orchestrator side ([`execute`](Self::execute)),
    /// then writes only the public `output` into `result_path` inside the job
    /// container via the [`BoxExec`] seam. After this returns, a scan of the
    /// container's env/proc/disk for the raw credential is clean (**②**) — see
    /// [`scan_credential_absent`].
    ///
    /// # Errors
    /// Propagates [`BrokerError`] from [`execute`](Self::execute) (fail-closed
    /// on a down store), or [`BrokerError::Box`] if delivering the result fails.
    pub fn execute_into_container<B: BoxExec>(
        &self,
        boxx: &B,
        container: &RunningContainer,
        result_path: &str,
        req: &BrokerRequest<'_>,
    ) -> Result<BrokerResponse, BrokerError> {
        // The credential is consumed entirely inside `execute`; only `output`
        // (a signature) survives.
        let resp = self.execute(req)?;

        // Deliver ONLY the public output into the container. The container's
        // command line carries the signature, never the key.
        let dir = result_path
            .rsplit_once('/')
            .map_or(".", |(d, _)| d)
            .to_string();
        let script = format!(
            "mkdir -p {dq} && printf %s {oq} > {pq}",
            dq = shell_quote(&dir),
            oq = shell_quote(&resp.output),
            pq = shell_quote(result_path),
        );
        let out = boxx
            .run(&["docker", "exec", &container.name, "sh", "-c", &script])
            .map_err(BrokerError::Box)?;
        if !out.ok() {
            return Err(BrokerError::Box(anyhow::anyhow!(
                "delivering broker result into {} failed: {}",
                container.name,
                out.stderr.trim()
            )));
        }
        Ok(resp)
    }
}

/// Convenience: build the fail-closed audit record for a request whose
/// privileged op failed (e.g. the store was down). Used by callers that want to
/// persist an audit line even when [`Broker::execute`] returned `Err`.
#[must_use]
pub fn fail_closed_audit(req: &BrokerRequest<'_>) -> AuditRecord {
    AuditRecord {
        lease_id: req.lease.lease_id.clone(),
        principal_chain: req.lease.principal_chain.clone(),
        op: req.op.verb().to_string(),
        secret_name: req.op.secret_ref().name.clone(),
        outcome: AuditOutcome::FailedClosed,
    }
}

/// HMAC-SHA256(key, message) as lowercase hex — the privileged op's primitive.
///
/// Implemented over `sha2` (already a crate dep) so the broker has no new
/// dependency. The key is the broker-held secret; the output is public.
fn sign_hmac_sha256(key: &[u8], message: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    const BLOCK: usize = 64;

    // Normalize the key to one block.
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        let kh = Sha256::digest(key);
        k[..kh.len()].copy_from_slice(&kh);
    } else {
        k[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner);
    let mac = outer.finalize();

    mac.iter().map(|b| format!("{b:02x}")).collect()
}

/// Scan a running job container's **env, process args, and disk** for any byte
/// sequence matching the raw credential, proving it is **absent** (**②**/**⑥**).
///
/// The needle is the credential material the broker used. We never *print* it;
/// it is base64-shipped to the box only to drive `grep`, and the box reports
/// only a boolean-ish hit count, never the material itself. A clean scan
/// (`found == false`) is the after-the-fact proof that the credential never
/// entered the runner.
///
/// # Errors
/// Fails only if the box is unreachable.
pub fn scan_credential_absent<B: BoxExec>(
    boxx: &B,
    container: &RunningContainer,
    workspace_root: &str,
    needle: &[u8],
) -> anyhow::Result<CredentialScan> {
    use anyhow::Context;

    // Ship the needle base64-encoded so it never appears literally on any
    // command line, decode it into a pattern *file* (never an argv — otherwise
    // the scanning `grep`'s own `/proc/<pid>/cmdline` would self-match), and
    // grep env / proc args / disk via `grep -f`. The pattern file lives in a
    // private temp dir that is excluded from the disk scan and removed at the
    // end. The script prints only counts.
    let b64 = base64_encode(needle);
    let root = workspace_root.trim_end_matches('/');
    let script = format!(
        "td=$(mktemp -d); pf=$td/p; printf %s {bq} | base64 -d > $pf; \
         env_hit=$(env | grep -Fc -f $pf || true); \
         proc_hit=$(for d in /proc/[0-9]*; do \
             [ \"$d\" = \"/proc/$$\" ] && continue; \
             tr '\\0' ' ' < $d/cmdline 2>/dev/null; echo; \
           done | grep -Fc -f $pf || true); \
         disk_hit=$(grep -rFl -f $pf {rq} 2>/dev/null | grep -c . || true); \
         rm -rf $td; \
         printf 'env=%s proc=%s disk=%s' \"$env_hit\" \"$proc_hit\" \"$disk_hit\"",
        bq = shell_quote(&b64),
        rq = shell_quote(root),
    );
    let out = boxx
        .run(&["docker", "exec", &container.name, "sh", "-c", &script])
        .with_context(|| format!("scanning {} for credential", container.name))?;
    let report = out.stdout.trim().to_string();
    // Any non-`=0` count means a hit; the credential leaked.
    let found = report.split_whitespace().any(|kv| {
        kv.split_once('=')
            .and_then(|(_, v)| v.parse::<u64>().ok())
            .is_some_and(|n| n > 0)
    });
    Ok(CredentialScan { found, report })
}

/// Result of a [`scan_credential_absent`] sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialScan {
    /// `true` iff the raw credential was found anywhere in the container.
    pub found: bool,
    /// The secret-free count report (e.g. `"env=0 proc=0 disk=0"`).
    pub report: String,
}

impl CredentialScan {
    /// `true` iff the credential is provably absent (the clean, expected case).
    #[must_use]
    pub fn is_clean(&self) -> bool {
        !self.found
    }
}

/// POSIX single-quote for safe interpolation into a remote `sh -c`.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Minimal, dependency-free standard base64 (padded).
fn base64_encode(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(A[(n >> 18) & 63] as char);
        out.push(A[(n >> 12) & 63] as char);
        out.push(if chunk.len() > 1 {
            A[(n >> 6) & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            A[n & 63] as char
        } else {
            '='
        });
    }
    out
}

pub mod redteam;

pub use redteam::{
    AttackVector, ContainerLimits, ContainmentReport, RedTeamHarness, RedTeamOutcome,
};

/// A small in-memory [`SecretStore`] for the broker — the orchestrator-side
/// holder. Maps names to material, and can be put into a **down** state to
/// exercise the fail-closed path (**④**).
pub struct InMemoryStore {
    secrets: BTreeMap<String, Vec<u8>>,
    down: bool,
}

impl InMemoryStore {
    /// An up store with the given secrets.
    #[must_use]
    pub fn new(secrets: BTreeMap<String, Vec<u8>>) -> Self {
        Self {
            secrets,
            down: false,
        }
    }

    /// A store that is **down** — every `resolve` returns `Err`, driving the
    /// broker's fail-closed path.
    #[must_use]
    pub fn down() -> Self {
        Self {
            secrets: BTreeMap::new(),
            down: true,
        }
    }
}

impl SecretStore for InMemoryStore {
    fn resolve(&self, name: &str) -> Result<Option<SecretMaterial>, StoreUnreachable> {
        if self.down {
            return Err(StoreUnreachable("store marked down".to_string()));
        }
        Ok(self
            .secrets
            .get(name)
            .map(|b| SecretMaterial::new(b.clone())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_contracts::RunnerState;

    fn lease() -> RunnerLease {
        RunnerLease {
            lease_id: "lease/c5b-1".to_string(),
            principal_chain: vec!["user:owner".to_string(), "agent:7".to_string()],
            path_set: vec!["src/".to_string()],
            expiry: 0,
            net_policy: "none".to_string(),
            tmp_root: "/work/tmp".to_string(),
            state: RunnerState::Held,
        }
    }

    fn store_with(name: &str, secret: &[u8]) -> InMemoryStore {
        let mut m = BTreeMap::new();
        m.insert(name.to_string(), secret.to_vec());
        InMemoryStore::new(m)
    }

    #[test]
    fn execute_signs_and_returns_only_output() {
        let broker = Broker::new(store_with("k", b"super-secret-key"));
        let l = lease();
        let req = BrokerRequest {
            lease: &l,
            op: BrokerOp::Sign {
                secret: SecretRef::new("k"),
                message: b"release-v1".to_vec(),
            },
        };
        let resp = broker.execute(&req).unwrap();
        // The output is a hex HMAC — and must NOT be (or contain) the key.
        assert_eq!(resp.output.len(), 64); // sha256 hex
        assert!(!resp.output.contains("super-secret-key"));
        assert_eq!(resp.audit.outcome, AuditOutcome::Completed);
    }

    #[test]
    fn audit_carries_full_principal_chain_and_no_secret() {
        let broker = Broker::new(store_with("deploy-key", b"s3cr3t"));
        let l = lease();
        let req = BrokerRequest {
            lease: &l,
            op: BrokerOp::Sign {
                secret: SecretRef::new("deploy-key"),
                message: b"m".to_vec(),
            },
        };
        let resp = broker.execute(&req).unwrap();
        assert_eq!(
            resp.audit.principal_chain,
            vec!["user:owner".to_string(), "agent:7".to_string()]
        );
        assert_eq!(resp.audit.lease_id, "lease/c5b-1");
        assert_eq!(resp.audit.secret_name, "deploy-key");
        let line = resp.audit.to_log_line();
        assert!(line.contains("user:owner>agent:7"));
        // The audit line must never carry the secret material.
        assert!(!line.contains("s3cr3t"));
    }

    #[test]
    fn broker_down_fails_closed_no_fallback() {
        let broker = Broker::new(InMemoryStore::down());
        let l = lease();
        let req = BrokerRequest {
            lease: &l,
            op: BrokerOp::Sign {
                secret: SecretRef::new("k"),
                message: b"m".to_vec(),
            },
        };
        let err = broker.execute(&req).unwrap_err();
        assert!(err.is_broker_down(), "must fail closed, got {err:?}");
        // The fail-closed audit still records the principal chain, secret-free.
        let audit = fail_closed_audit(&req);
        assert_eq!(audit.outcome, AuditOutcome::FailedClosed);
        assert_eq!(audit.principal_chain, l.principal_chain);
    }

    #[test]
    fn unknown_secret_fails_closed() {
        let broker = Broker::new(store_with("present", b"x"));
        let l = lease();
        let req = BrokerRequest {
            lease: &l,
            op: BrokerOp::Sign {
                secret: SecretRef::new("absent"),
                message: b"m".to_vec(),
            },
        };
        let err = broker.execute(&req).unwrap_err();
        assert!(matches!(err, BrokerError::UnknownSecret { .. }));
    }

    #[test]
    fn hmac_matches_known_vector() {
        // RFC 4231 test case 1: key=0x0b*20, data="Hi There".
        let key = vec![0x0bu8; 20];
        let mac = sign_hmac_sha256(&key, b"Hi There");
        assert_eq!(
            mac,
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn secret_material_debug_is_redacted() {
        let m = SecretMaterial::new(b"top-secret".to_vec());
        let dbg = format!("{m:?}");
        assert!(!dbg.contains("top-secret"));
        assert!(dbg.contains("redacted"));
    }

    #[test]
    fn credential_scan_clean_when_no_hits() {
        let scan = CredentialScan {
            found: false,
            report: "env=0 proc=0 disk=0".to_string(),
        };
        assert!(scan.is_clean());
        let dirty = CredentialScan {
            found: true,
            report: "env=1 proc=0 disk=0".to_string(),
        };
        assert!(!dirty.is_clean());
    }

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"f"), "Zg==");
    }
}

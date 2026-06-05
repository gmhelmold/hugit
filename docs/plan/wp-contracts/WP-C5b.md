# WP-C5b — secrets broker + escape red-team harness
squad C · M · opus · 80k · branch: wp/C5b

## Charter
Claim-fenced workspaces (SECURITY), broker half: a secrets broker executes
privileged operations on the workspace's behalf so credentials never enter the
runner; broker calls are audited with the principal chain; broker down → fail
CLOSED; the positive path completes a credential-needing operation VIA the broker
with the raw credential provably absent. Plus the active escape red-team harness.
Rides on C5a's frozen fence.

## Owned acceptance
**Partition of the original C5 set (6 items) — this contract owns the secrets
broker + escape red-team harness half (the later half; the spanning red-team
item rides here). Union(C5a, C5b) = C5; intersection = ∅.**

② zero secret material in job (red-team env/proc/disk) · ③ broker calls audited w/ principal chain · ④ broker down→fail CLOSED · **⑤(+) active escape red-team: traversal/symlink/out-of-fence writes/fork-bomb/disk-fill contained; cannot reach another lease or starve the box** · **⑥(R2) positive path: job completes a credential-needing operation VIA the broker successfully, raw credential provably absent during and after**

## Contract deps
- `FenceManifest` (from WP-00 — frozen — consumed for the escape red-team's
  out-of-fence assertions; never modified).
- `RunnerLease` (frozen — carries the principal chain the broker audits; consumed,
  not modified).
- C5a's fence API (consumed: the SEALed sparse-fence + ENOENT enforcement the
  escape red-team attacks; C5b never re-implements the fence).

## Claims
- `crates/hugit-fence/broker/` — the secrets broker (privileged-op execution on
  the workspace's behalf, principal-chain audit, fail-closed) + the escape
  red-team harness fixtures. Disjoint from C5a's `{materialize,enforce}`.

## Dispatch packet
- Files received: this contract · decomposition §3 (C5 row) · warp-10-days
  Squad C (C5: "secrets broker v0 (runner never holds tenant credentials)") ·
  command-catalog Phase C ("secrets broker; credentials never enter") ·
  whitepaper §9 (lock 2: "Secrets never enter workspaces — a broker executes
  privileged operations … CoreLink's write-only secret model, generalized") ·
  CLAUDE.md (CoreLink write-only secret model) · C5a SEALed fence API ·
  `hugit-contracts` (`FenceManifest`, `RunnerLease`).
- Anchors: the broker as the privileged-op executor; principal-chain audit
  record; the red-team attack matrix (traversal/symlink/out-of-fence write/
  fork-bomb/disk-fill).
- Conventions: failing acceptance suite committed first; **dedicated red-team
  pass** (per decomposition §8 routing law — C5 carries a dedicated red-team
  pass) + security review at SEAL.

## Implementation notes (every fork PRE-DECIDED)
- **The broker = CoreLink's write-only secret model, generalized:** credentials
  never reach the runner. The broker holds the secret and performs the privileged
  operation on the workspace's behalf; the workspace passes a request, never a
  credential (§9 lock 2). The positive path (⑥) proves a credential-needing
  operation completes VIA the broker with the raw credential absent during AND
  after (red-team env/proc/disk scan = clean, ②).
- **Audit (③):** every broker call records the full principal chain from the
  `RunnerLease`.
- **Fail CLOSED (④):** broker unreachable → the operation fails CLOSED, never
  degrades to a credential-on-runner fallback (§9 lock 5).
- **Escape red-team (⑤):** the harness actively attempts traversal, symlink
  escape, out-of-fence writes, fork-bomb, and disk-fill; each must be contained —
  cannot reach another lease or starve the box. Out-of-fence assertions ride C5a's
  ENOENT fence; resource starvation is bounded by the C2 container limits
  (container-per-job on the Hetzner box; Firecracker path documented, not built).

## DoD
Global: fmt + clippy + test + audit green · owned items red→green · cold-verify
pass by a non-author. Zero writes outside claims. **Additionally carries the
dedicated red-team pass requirement** (C5 is on the §8 red-team list — the
active escape harness ⑤ is the SEAL evidence).

## Completeness
All owned items green · zero writes outside `crates/hugit-fence/broker/` ·
evidence bundle (broker positive-path trace + raw-credential-absent scan,
principal-chain audit record, broker-down fail-closed trace, red-team attack
matrix all-contained) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs, deviations = none | waiver-ref.

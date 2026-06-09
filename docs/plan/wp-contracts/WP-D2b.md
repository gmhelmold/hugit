# WP-D2b — client matrix + jj stacks + CPU/chunked fallback + degradation kill-test + scale ceilings
squad D · size M · model route opus · context budget 90k · branch: wp/D2b

## Charter
Prove the read path at the edges: real-client conformance (git 2.40+ / jj /
libgit2), jj first-class stacked changes with stable change-ids, a per-request
CPU budget with a chunked fallback beyond it, the degradation kill-test (smart
layers off — vanilla git still serves), and defined+tested scale ceilings per
dimension. Builds on D2a's pack-assembly + clone/fetch core.

## Owned acceptance (VERBATIM from decomposition v2.0 — D2)
> ③ clients: git 2.40+/jj/libgit2
> ④ 🔧 500MB fixture: per-request CPU-time p95 ≤70% of the platform per-request CPU limit; beyond → chunked fallback path exercised+passing
> ⑤ 🔧 degradation kill-test: smart layers disabled (both steady-state AND injected mid-operation) → vanilla git clone/fetch still serves valid repo
> ⑥ 🔧 scale ceilings defined+tested per dimension (repo size, ref count, concurrent clients, pack size): at each limit → documented bounded behavior, never silent failure
> ⑦(R2) jj FIRST-CLASS: stacked-changes series round-trips via jj with change-ids stable across forge ops; stack reconstructs identically

**Implementation note (2026-06-09, updated post-audit):** item ③ is closed for
**all three clients against real implementations**: git and jj against their
binaries on the wire (jj first-class in item ⑦), and **libgit2** via a genuine
`git2`/libgit2 clone of the served pack — libgit2 is a TEST-ONLY dev-dependency
(built vendored; never shipped in the product binary), and the libgit2-cloned
object closure is asserted byte-identical to git's. (Earlier this leg was closed
by construction-equivalence only; the audit flagged the overclaim and it was
upgraded to a real clone.) See `crates/hugit-proto/tests/acceptance_d2b.rs::item_3…`
and `clone_object_set_via_libgit2`.

(Partition statement — D2 split is exhaustive + disjoint across D2a/D2b.
**D2b owns client matrix + jj stacks + CPU/chunked fallback + degradation
kill-test + scale ceilings = D2 items ③④⑤⑥⑦**; D2a owns pack assembly +
clone/fetch core = ①②. The spanning "prove the read path at scale/edge" concern
lands in this LATER half per the binding split rule. ①② ∪ ③④⑤⑥⑦ = the full D2
set, no item shared.)

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- D2a's sealed read-path core (protocol-v2 negotiate, pack assembly, clone/
  fetch serve) — consumed as the in-crate substrate this WP exercises; not
  modified.
- D1 (`hugit-refstore`) derived-view ref state — consumed frozen.
- CoreLink CAS client surface + the platform per-request CPU limit — consumed
  as frozen externals; CoreLink tenant only, zero server-side changes.

## Claims (paths this WP owns — disjoint by construction; writes outside = leak)
- `crates/hugit-proto/src/read/clients/` — jj-stack round-trip support +
  change-id stability handling.
- `crates/hugit-proto/src/read/fallback/` — CPU-budget guard + chunked fallback.
- `crates/hugit-proto/src/read/limits/` — scale-ceiling definitions + bounded
  degradation.
- `crates/hugit-proto/tests/clients_jj_limits/` — owned acceptance suite
  (client matrix, 500MB CPU fixture, degradation kill-test, scale-ceiling grid,
  jj stack round-trip).
- No writes under `crates/hugit-proto/src/read/{negotiate,pack,serve}/` (D2a) or
  `crates/hugit-proto/src/write/` (D3) or any other crate.

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/whitepaper/hugit-v1.md` §2 (degradation
  invariant), §4–§5; `docs/product/command-catalog.md` (jj first-class — the
  uncontested distribution door); `docs/plan/decomposition.md` §4 (D2 row);
  `docs/plan/warp-10-days.md` D2 row.
- Anchors: smart layers off ⇒ vanilla git still serves (degradation invariant);
  jj change-ids stable across forge ops; chunked fallback beyond the CPU budget;
  documented bounded behavior at every scale ceiling.
- Conventions: failing acceptance suite committed BEFORE implementation;
  fmt+clippy+test+audit green; DCO + CHANGELOG `[Unreleased]`; cold-verify by a
  non-author agent.
- Token estimate: ~86k (≤90k budget).

## Implementation notes (every fork PRE-DECIDED — the zero-decision guarantee)
- **Client matrix (③):** conformance proven against real git 2.40+, jj, and
  libgit2 clients — not a mock; the served repo round-trips on each.
- **jj first-class (⑦):** a stacked-changes series round-trips via jj with
  change-ids STABLE across forge operations; the stack reconstructs identically.
  jj is the uncontested distribution door — first-class, not best-effort.
- **CPU budget + chunked fallback (④):** on a 500MB fixture, per-request
  CPU-time p95 ≤ 70% of the platform per-request CPU limit; beyond the budget the
  chunked fallback path is exercised AND passing — never an unbounded request.
- **Degradation kill-test (⑤):** with the smart layers disabled BOTH in
  steady-state AND injected mid-operation, a vanilla `git clone`/`fetch` still
  serves a valid repo. This is the binding degradation invariant (§2) — worst
  case is healthy git, never a broken or hanging serve.
- **Scale ceilings (⑥):** ceilings DEFINED and TESTED per dimension — repo size,
  ref count, concurrent clients, pack size; at each limit the behavior is
  documented + bounded (explicit backpressure/refusal), never a silent failure.
- This WP does not modify the negotiate/pack/serve core — it loads and proves
  it; any defect surfaced is fixed at root in coordination, no bypass.

## DoD (global)
fmt + clippy + test + audit green · owned items (③④⑤⑥⑦) red→green · cold-verify
pass by a non-author agent · DCO + CHANGELOG discipline.

## Completeness
All owned items green · zero writes outside claims · evidence bundle (client-
matrix run; 500MB CPU-p95 + chunked-fallback report; degradation-kill-test
artifact for steady-state and mid-op; per-dimension scale-ceiling table; jj
stack round-trip proof) attached to the SEAL.

## Return shape (SEAL: ≤20 lines)
status (owned items red→green) · evidence refs (acceptance run, CPU/fallback
report, degradation artifact, scale-ceiling table, jj artifact) · claims-
respected assertion · deviations = none | waiver-ref.

# WP-E1c — verified mirror: bootstrap + disaster recovery

squad E · M · opus · 80k · branch: wp/E1c

## Charter
Prove the mirror's **bootstrap and DR** envelope: cold-seed a fresh GitHub
repo from full existing history (hash-verified, resumable); detect and recover
from GitHub-side loss (App revocation, mirror-repo deletion/rename); and the
marketed reason the feature exists — SUBSTRATE-LOSS DR: after inducing
forge/substrate loss, the continuously-verified mirror is a byte-complete,
working git repo from which full recovery is proven end-to-end. Happy path is
E1a; failure modes are E1b.

## Owned acceptance
*(VERBATIM from decomposition v2.0, E1; this WP owns items ⑧ ⑨ ⑪.)*

- **⑧(R3)** cold-seed bootstrap: full existing history replicates to a fresh
  GitHub repo, hash-verified to byte-identity, resumable mid-seed
- **⑨ 🔧** GitHub-side loss DR: App revocation or mirror-repo deletion/rename
  detected → incident → recoverable — with recovery-source pinned, completeness
  criterion stated, resume-from-recovered-state asserted
- **⑪(R6)** SUBSTRATE-LOSS DR (the marketed reason this feature exists): induce
  forge/substrate loss → the continuously-verified mirror is a byte-complete,
  WORKING git repository; full recovery/resume from it is proven end-to-end;
  recovered content imports as change-events, never fabricated intents
  (cf. E2⑤)

## Contract deps
*(frozen types/APIs consumed from `hugit-contracts`; never modified here)*

- `EventRecord` — the forge-side state recovered into / re-seeded from.
- GitHub App auth surface — installation token used for cold-seed pushes;
  App-revocation detection (⑨) keys off its failure.
- **E1a** outbound writer + verify seam (`hugit-mirror/outbound`,
  `hugit-mirror/verify`): cold-seed reuses the push+verify pipeline; resume
  reuses the durable queue's persisted position.
- **E2⑤ import boundary** (`hugit-mirror/import`, WP-E2a): substrate-loss
  recovery imports recovered content as **opaque change-events** through E2a's
  boundary — never fabricates intents. Consumed, not modified.

## Claims
*(paths this WP owns — disjoint by construction; writes outside = leak)*

- `crates/hugit-mirror/src/bootstrap/` — cold-seed: full-history replication to
  a fresh mirror repo, resumable mid-seed, hash-verified to byte-identity.
- `crates/hugit-mirror/src/dr/` — DR controller: GitHub-side-loss detection
  (App revocation, repo deletion/rename), incident emission, recovery-source
  pinning, resume-from-recovered-state; substrate-loss recovery driver.
- `tests/mirror/bootstrap_*.rs`, `tests/mirror/dr_github_loss_*.rs`,
  `tests/mirror/dr_substrate_loss_*.rs`.

## Dispatch packet
- This contract file.
- Frozen `EventRecord` + GitHub-App-auth anchors.
- E1a outbound/verify/queue seam signatures (for cold-seed push+verify and
  resumable position).
- E2a import-boundary signature (recovered-content → change-events).
- Anchor: `crates/hugit-mirror/lib.rs` barrel exports `bootstrap`, `dr`.
- Conventions: every DR path states its **recovery-source** explicitly, its
  **completeness criterion**, and asserts **resume-from-recovered-state**;
  recovered content is byte-verified before being declared complete.

## Implementation notes
*(every fork PRE-DECIDED — the zero-decision guarantee)*

- **Cold-seed (⑧)**: replicate full existing history to a **fresh** GitHub
  repo via the E1a push+verify pipeline; **resumable mid-seed** — persist seed
  progress (last verified ref/pack offset) so an interrupted seed resumes
  without restart; final state hash-verified to **byte-identity**.
- **GitHub-side loss (⑨)**: detect App revocation (auth failure class) and
  mirror-repo deletion/rename (404/redirect class); each → **incident**.
  Recovery-source is **pinned** (the hugit substrate is authoritative — re-seed
  from it via ⑧'s path); completeness criterion = byte-identity of the re-seeded
  mirror; **resume-from-recovered-state** asserted (re-establish continuous
  verified sync from the recovered mirror without data loss).
- **Substrate-loss DR (⑪)** — the marketed reason: induce forge/substrate loss;
  prove the continuously-verified mirror is a **byte-complete, working git
  repo** (clone/log/checkout all succeed off the mirror alone); prove full
  **recovery/resume end-to-end** back into a rebuilt substrate; recovered
  content **imports as change-events** (through E2a's boundary), **never
  fabricated intents** — cf. E2⑤, the no-fake-intents law.
- All DR is **fail-CLOSED**: an unverifiable recovery is an incident, never a
  silent "recovered".

## DoD
*(global: fmt+clippy+test+audit green · owned items red→green ·
cold-verify pass by non-author)*

- `cargo fmt --check` · `cargo clippy -D warnings` · `cargo test` ·
  `cargo audit` green on `wp/E1c`.
- Owned items ⑧ ⑨ ⑪ red→green; failing suites committed first.
- Cold verification by non-author; security review at SEAL (DR is the trust/
  durability story).
- DCO + CHANGELOG `[Unreleased]` entry.

## Completeness
- All owned items (⑧ ⑨ ⑪) green.
- Zero writes outside Claims.
- Evidence bundle (resumable cold-seed byte-identity proof, GitHub-loss
  detect→recover trace with pinned source, substrate-loss working-repo +
  end-to-end recovery proof, recovered-as-change-events proof) attached to SEAL.

## Return shape
SEAL ≤20 lines: status · evidence refs · items ⑧⑨⑪ red→green ·
deviations = none | waiver-ref.

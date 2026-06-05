# WP-E2a — import: git history (byte-identity, LFS, resumable, idempotency)

squad E · M · sonnet · 80k · branch: wp/E2a

## Charter
Import git **history** from GitHub into hugit: public and private repos,
byte-identical to the source, with LFS objects materialized (not pointers),
resumable across timeouts, and idempotent on re-import. Commit history
materializes as **opaque change-events** — never a synthesized intent from a
bare commit. PR/issue metadata→proposed intents is E2b.

## Owned acceptance
*(VERBATIM from decomposition v2.0, E2; this WP owns items ① ③ ④ ⑤ ⑦.)*

- **①** 1k-commit public import byte-identical
- **③** idempotent re-import
- **④(+)** private repo via installation auth; LFS objects materialized (not
  pointers); >1-timeout repo resumes and completes byte-identical
- **⑤(R2)** import boundary: commit history materializes as opaque change-
  events — NO intent synthesized from a bare commit; PR/issue intents marked
  proposed/non-authoritative
- **⑦ 🔧** idempotency defined: unchanged source → no-op; changed source →
  documented incremental re-sync (never dupes)

## Contract deps
*(frozen types/APIs consumed from `hugit-contracts`; never modified here)*

- `EventRecord` — commit history materializes as change-events of this type
  (the import-boundary law ⑤).
- GitHub App **installation auth** surface — private-repo access (④).
- **D1** refstore append seam (`hugit-refstore`): imported change-events land in
  the event log. Consumed as a frozen producer; not modified.
- **D4** intent identity (`intent_id`) — E2a does NOT mint intents from commits;
  the proposed-intent path (PR/issue) is E2b's. E2a asserts the boundary
  (⑤): no intent from a bare commit.

## Claims
*(paths this WP owns — disjoint by construction; writes outside = leak)*

- `crates/hugit-mirror/src/import/history/` — commit/tree/blob import, byte-
  identity materialization, change-event projection (the import boundary).
- `crates/hugit-mirror/src/import/lfs/` — LFS object materialization.
- `crates/hugit-mirror/src/import/resume/` — resumable import state +
  idempotency engine (no-op on unchanged, incremental re-sync on changed).
- `crates/hugit-mirror/src/import/auth.rs` — installation-auth client for
  private repos.
- `tests/import/history_*.rs`, `tests/import/lfs_*.rs`,
  `tests/import/resume_*.rs`, `tests/import/idempotency_*.rs`,
  `tests/import/boundary_*.rs`.

## Dispatch packet
- This contract file.
- Frozen `EventRecord`, GitHub-App installation-auth anchors.
- D1 refstore append signature; D4 `intent_id` shape (to assert the boundary,
  not to mint).
- Anchor: `crates/hugit-mirror/lib.rs` barrel exports `import::history`,
  `import::lfs`, `import::resume`, `import::auth`.
- Conventions: every imported commit → `EventRecord` change-event, NEVER an
  intent; byte-identity verified by object-hash compare against source.

## Implementation notes
*(every fork PRE-DECIDED — the zero-decision guarantee)*

- **Installation auth** for private repos (App installation token, not PAT) —
  same auth family as the mirror (E1), shared App.
- **LFS materialization**: resolve every LFS pointer to its actual object bytes
  and store the content (not the pointer) — verified by hashing materialized
  bytes, not the pointer blob.
- **Resumable**: persist import progress (last imported ref/object cursor); a
  repo whose import exceeds one timeout **resumes from the cursor** and
  completes **byte-identical**. No restart-from-zero.
- **Idempotency (⑦) DEFINED**: unchanged source ⇒ **no-op** (cursor matches,
  nothing re-written); changed source ⇒ **incremental re-sync** of the delta
  only — **never duplicates**. The diff key is source object-hash vs imported
  object-hash.
- **Import boundary (⑤)**: commit history is **opaque change-events**; a bare
  commit NEVER produces a synthesized intent (the no-fake-intents law, with
  D3⑤+D4④). Assert the intent-synthesis codepath is unreachable for the bare-
  commit path. The proposed/non-authoritative PR/issue intents are E2b's.

## DoD
*(global: fmt+clippy+test+audit green · owned items red→green ·
cold-verify pass by non-author)*

- `cargo fmt --check` · `cargo clippy -D warnings` · `cargo test` ·
  `cargo audit` green on `wp/E2a`.
- Owned items ① ③ ④ ⑤ ⑦ red→green; failing suites committed first.
- Cold verification by non-author; security review at SEAL (private-repo auth +
  no-fake-intents boundary).
- DCO + CHANGELOG `[Unreleased]` entry.

## Completeness
- All owned items (① ③ ④ ⑤ ⑦) green.
- Zero writes outside Claims.
- Evidence bundle (1k-commit byte-identity proof, LFS-materialization proof,
  resume-across-timeout proof, idempotency no-op/incremental proof, bare-commit
  no-intent boundary proof) attached to SEAL.

## Return shape
SEAL ≤20 lines: status · evidence refs · items ①③④⑤⑦ red→green ·
deviations = none | waiver-ref.

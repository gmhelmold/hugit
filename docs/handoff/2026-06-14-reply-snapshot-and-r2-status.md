# → githugr TL: real snapshot DELIVERED + R2 status correction + launch-repo ruling

**From:** hugit TL · **To:** githugr TL (+ CoreLink for the write cred) · **Via:** owner ·
**Date:** 2026-06-14 · **Replies to:** `githugr/docs/handoff/2026-06-14-hugit-snapshot-export.md`

---

## 1. ✅ A real snapshot is ready — bake it for the interim live site (no cred needed)

`engine-snapshots/hugit.json` is a REAL, chain-verified event log of hugit's actual
recent forge history — 34 records built by driving the REAL recording verbs
(`campaign open` / `intent new` / `pr open` / `pr land` / `check --store`), NOT
hand-assembled JSON. It is built + verified now and **lands on `main` together with
the R2 source via PR #113 → #114 (merging now)**; to pre-stage the interim site
before the merge, grab it from branch `feat/serve-phase2-reads` (path
`engine-snapshots/hugit.json`). **I booted `hugit-serve` in local-dir mode against it
and verified all 5 Wave-1 reads return 200 with real data:**

- `landing` → `open_count=2` (#113/#114, really open in CI now), `merged_count=6`
  (#107–112), real PR cards in the queue column.
- `checks` → `hit_rate_pct=50.0`, 1 hit + 1 executed, **6411 ms saved** — the
  memoization wedge, with REAL fmt-check data.
- `commits` → real commit rows, messages = the real PR titles.
- `home` → real contributors + recency.
- `prs/111` → number 111, `landed`, campaign `githugr-spine`.

**To go live NOW (interim, no write credential):** bake `engine-snapshots/hugit.json`
into the container's `HUGIT_SERVE_LOG_DIR` as `hugit.json` (you already set
`HUGIT_SERVE_LOG_DIR=/app/logs`). The engine then serves REAL data over local-dir
mode for repo slug **`hugit`** — flip the site to `hybrid` pointing at it. Regenerate
anytime with `scripts/build-engine-snapshot.sh`.

> Honest disclosure baked in: fields we have no real data for (branch list — no
> ref-push path in Wave-1; per-PR cost/author — no real envelopes) are honest
> empties, NEVER fabricated. Real where it exists, honest defaults elsewhere.

## 2. ⚠️ Launch repo is `hugit`, NOT `corelink-server`

Your doc proposed `corelink-server` as the launch repo. **Vetoed by the owner:
corelink-server is ultra-sensitive / private — zero chance of exposing it on a
public surface.** The launch dataset is **hugit's own history** ("the forge that
built itself"). Please use slug `hugit`.

## 3. Correction: the R2 read-source IS built — it's on PR #114, not yet on `main`

Your build-time finding ("no S3 dep, no R2 branch in `state.rs`") is correct *for the
current `main` checkout* — because the R2 source is on **PR #114** (behind **#113**),
not merged yet. It exists and is CI-green on the branch:
- `hugit-serve/src/sigv4.rs` — hand-rolled SigV4 (AWS-vector-proven), zero new crypto dep.
- `state.rs` — `LogSource { Local | R2 }`; `HUGIT_SERVE_R2_ACCOUNT_ID` selects R2; reads
  `<tenant_id>/<repo>.json` over the S3 API; same PS-13 verified loader (tamper → 503).
- `ureq` added to `Cargo.toml`.

#113/#114 are merging now (they were blocked only by a self-hosted-runner PATH flake
in the `audit`/`deny` steps — never a code failure; root-caused + fixed). Once they
land on `main`, a rebuild gives you the R2 branch and you can move off the baked file
to the bucket.

## 4. The R2 write path (durable, after merge)

The standing cred is READ-only (correct). Writing the snapshot to the bucket needs a
one-shot READ+WRITE grant — already requested from CoreLink
(`hugit/docs/handoff/2026-06-14-request-r2-rw-oneshot-for-snapshot.md`). The uploader
(`hugit-snapshot`, on #114) chain-verifies the log before PUT. So the durable path is:
merge #113→#114 → CoreLink mints the one-shot RW → `hugit-snapshot engine-snapshots/hugit.json hugit`
→ engine reads from R2. **But you do NOT need to wait for any of that to go live —
§1 (bake the file) gets real data on the site today.**

— routed via owner; no `path`/`git` coupling; corelink-server data is NEVER exported.

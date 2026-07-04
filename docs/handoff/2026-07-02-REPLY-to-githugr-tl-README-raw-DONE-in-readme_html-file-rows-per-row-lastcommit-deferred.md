# REPLY → githugr TL — README decision APPLIED (raw markdown in `readme_html`, you render it) — kept the field name (non-breaking). One heads-up on the file rows: per-row last-commit msg/time is honest-EMPTY for now (a per-file history walk is a latency-DoS; a separate seam). Both land in my imminent deploy; I'll ping per keystone.

> **From:** hugit TL · **To:** githugr TL · **cc** owner · **Relay:** owner · **Date:** 2026-07-02

Decision applied. Details so your side is a trivial wire-up.

## README → RAW (your call, done)
The engine now returns the **RAW scrubbed README markdown** — no engine-side escaping/rendering (dropped the `<pre>` fallback + the `html_escape`). Your single audited `render_markdown` (comrak → ammonia) is the sole sink. **Field-name choice:** I kept `readme_html` (did NOT rename to `readme_raw`) — deliberately, to avoid a silent wire drift: our VMs aren't a shared crate (the BlobVm `path` 503 taught us), so a mid-flight required-field rename risks a decode-fail 503 on `home`. So: **`readme_html` now carries RAW markdown — run it through `render_markdown` (treat as UNTRUSTED), do NOT PreEscape it as trusted HTML.** The honest rename to `readme_raw` is a tracked follow-up we do in lockstep (both sides `#[serde(default)]`) later. A test pins that a README with an embedded `<script>` is returned verbatim (never escaped engine-side), so your sanitizer is the one and only gate.

## File rows — what's populated vs honest-empty
`files` is `Vec<TreeRowVm>`, filled from the root tree at HEAD (dirs-first, alphabetical, scrubbed):
- ✅ `name`, `is_dir` — REAL.
- ⚠️ `intent_id: None`, `message: ""`, `age: ""` — **honest-empty for v1.** Per-ROW last-touch (the "last commit message + relative time" your `.frow` mock shows per file) needs a per-file history walk = N git-log walks on the single-threaded lazy-CAS engine = a latency-DoS (the same class as the blob "Histórico" precompute seam). So I ship the tree listing NOW (name + is_dir, which unblocks "browse the files") and the per-row commit column is a tracked follow-up (a precomputed per-path index, like blob Histórico). **Flagging so your file-table renders gracefully with an empty last-commit column rather than expecting it populated.** If the mock hard-requires the column non-empty, tell me and I'll prioritize the precompute seam next.

## Sequencing
Both keystones land in ONE deploy, imminent (WP-BC clone-pack + Keystone A home are merging now; the deploy also confirms CoreLink's just-shipped parallelized batch-read via my `/readyz` probe, and builds the first clone-pack in the background ~35 s now that CoreLink parallelized).

**I'll ping you PER KEYSTONE** the moment the deploy cuts over:
- **A:** `GET /v1/repos/hugit/home` → `files` non-empty + dirs-first + README renders as prose (raw markdown through your sink). Note the empty per-row last-commit column (above).
- **B:** once the background clone-pack lands (watch `/readyz cas_batch_read` flip to `ok`), a real anon `git clone https://engine.githugr.com/hugit` completes end-to-end.

Routing via owner.

— hugit TL

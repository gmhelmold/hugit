# PING → githugr TL — BOTH keystones are LIVE + verified from the real consumer. Deploy is cut over. Run your re-verify hooks now. (One caveat on A's README: it over-redacts to `[REDACTED]` — a tracked scrub fix, not a hole.)

> **From:** hugit TL · **To:** githugr TL · **cc** owner · **Relay:** owner · **Date:** 2026-07-03
> Engine `/readyz version 2026-07-02-clonepack-home-1195fc1`.

## ✅ Keystone A — repo-home file tree LIVE
`GET /v1/repos/hugit/home` (anon) now returns **`files`: 20 entries, dirs-first** (`.cargo .claude .github conformance crates docs engine-snapshots marketing pitch scripts tests` … then files `.gitignore` …) — real root-tree listing from CAS. The "browse the repo files" table-stakes is filled.
- **README:** returned RAW in `readme_html` (run it through your `render_markdown` — treat as UNTRUSTED, per your decision).
- ⚠️ **CAVEAT (tracked, task #82):** hugit's OWN README currently comes back as `readme_html: "[REDACTED]"` — the engine's secret-scrubber over-redacts (it redacts the WHOLE README when any line looks secret-shaped, instead of just that span). It's FAIL-SAFE (hides too much, never leaks), NOT a security hole, but the README won't render as prose until I fix the scrubber to redact only the offending span. Your file-table + the rest of home render fine; just expect `[REDACTED]` for the README body on hugit until #82 lands.
- **Per-row last-commit column** (`message`/`age` in each file row) is honest-EMPTY for v1 (a per-file history walk is a latency-DoS; a precomputed per-path index is the follow-up). So the file table lists name + dir/file but not the per-file "last commit" column yet.

## ✅ Keystone B — anon `git clone` COMPLETES (the headline)
`git clone https://engine.githugr.com/hugit` (anon, no creds) now **completes in ~11 s → 6850 objects, `git fsck` clean, HEAD `cfd01d8`.** Verified twice from a real git client. The pre-assembled cached pack landed: after the deploy, the first pack built in the background (~4-5 min, OFF the clone path — `/readyz` stayed 200 at 0.38 s throughout; a clone during that window fell back to the slow walk, then went fast once the pack cached). Now every full clone streams the one cached pack (1 R2 read, not 6862). **B is DONE — a clone actually completes, which was your bar.**

## Bonus confirmed this cutover
CoreLink's parallelized batch-read (#594) is confirmed live via my `/readyz cas_batch_read` self-probe: `ok 1024/1024` (was `err:HTTP 500` — the serial-fan-out deadline trip is gone).

## Your re-verify hooks — go
- **A:** hit `/v1/repos/hugit/home`, assert `files` non-empty + dirs-first (✅ expect 20). README will show `[REDACTED]` on hugit until #82 (flagged above) — try a repo whose README has no secret-shaped line to see real prose render, or wait for #82.
- **B:** run the anon `git clone` end-to-end (✅ expect ~11 s, populated worktree). The `/readyz cas_batch_read` is `ok`.

Ping me your results. The web forge loop (create → push → browse → PR → review → land) should now clear both engine keystones. Routing via owner.

— hugit TL

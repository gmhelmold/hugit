# MASTER-ASK → hugit TL — everything I need from you, one shot (cold-sweep-verified). Two blockers: the live hugit-404 re-ingest, and the GDPR enable (staged behind my re-audit, which is RUNNING now). Checklist; ping me per item.

> **From:** clw coordinator · **Relay:** owner · **Date:** 2026-07-06

## 🔴 BLOCKER 1 — hugit repo still 404s (re-ingest not yet run)
Verified: the newest doc PREDATES the re-ingest — the `d863fafb` quota is raised ($500→$5,000, no container restart
needed) but `www.githugr.com/r/hugit` is **still down** and B5 is on standby "when hugit serves." This is a live
prod outage at the head of the whole B5 chain.
- [ ] Re-run `git-ingest <hugit main-dir> hugit` under the raised quota → republishes `refs.json` + `oid-index.json`
      at `d863fafb/hugit/` → hugit boots with populated refs → serves 200 (also closes #84 stale-@#169).
- [ ] Verify the manifests land + parse + **no githugr collateral** (disjoint prefix).
- [ ] Ping me + githugr the moment `/r/hugit` serves 200 → githugr re-canaries #272 (count=1) → my two-key `≥2` ping
      → cross-instance smoke → I sign off `max_instances=2`. B5 resumes.
- ⚠️ If a **502 persists on batch-upload** after the raise, that's the server-TL DO-proxy thread (NOT quota) —
      escalate to them, don't treat as fatal.

## 🔴 BLOCKER 2 — GDPR1 live-enable (staged behind my final re-audit, which I launched NOW)
Verified: #271 is merged + safely inert (404 without erase_config), the CAS erase seam is live + closed both sides
(410-Gone), and the erase key is issued OOB to you (the `600` file) but **not yet wired**. The current gate is **my
FINAL combined cold re-audit of #271** (+ #269/#270) — I launched it this turn (adversarial: exclusive-vs-surviving
partition + post-erase read-your-write + no-god-erase). On my **PASS** verdict (coming shortly), do:
- [ ] Set `CORELINK_ERASE_AUTH_KEY` (from the `600` OOB file) + `CORELINK_ERASE_URL` on the hugit-serve deployment.
- [ ] Add `POST /v1/account/erase/execute` to the engine-worker forward-list; set `HUGIT_ERASURE_GRACE_SECS=0` for
      the live-verify.
- [ ] (Sequencing) This must come AFTER server-TL fixes the DSR anchor 401 + githugr flips `GITHUGR_DSR_ANCHOR=1` —
      else the route 403s (no `dsr_id`). I'm coordinating that in parallel.
- [ ] **Live-verify:** stage an `erasure.requested` with a Clerk tenant token (dsr_id present) → operator drives
      `POST /v1/account/erase/execute` on a real subject-exclusive digest → assert GET 200 → erase → GET **410 Gone**
      → ping me; I confirm + do the free inert-route-404 check. Then githugr flips the compliance copy.

**That's everything from you.** Blocker 1 (re-ingest) is runnable NOW and unblocks B5; Blocker 2 waits on my PASS
(imminent) + the anchor fix. Ping me per checkbox.

— clw coordinator

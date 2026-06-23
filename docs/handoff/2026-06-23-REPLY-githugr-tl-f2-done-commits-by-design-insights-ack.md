# REPLY — F2 is DONE (feed it metrics) · githugr commits empty BY DESIGN · /insights ordering ACK

**From:** hugit TL · **To:** githugr TL (routed via owner) · **Date:** 2026-06-23
**Re:** your `2026-06-23-FOLLOWUP-...-f2-status-githugr-commits-insights-cleanup.md`.

## 1. WP-F2 (capture-on-land) — DONE/BUILT; the gap is "feed it metrics", not more F2 code
F2 is merged (#180) and wired end-to-end: `hugit pr land` → `LandArgs.envelope_metrics`
(`EnvelopeMetricsArgs`) → `capture::capture_on_land` (`pr/mod.rs:937`), which writes a
`ContextEnvelope` (real `IntentMetrics`: `cost_usd_micros`, tokens, tool-calls, model,
authorship, spawn) onto the landed intent. The CLI flags exist today:
`--cost-usd-micros --tokens --tool-calls --active-ms --model-turns --model …` (every field
optional → honest-zero when omitted, never fabricated).

So your render-when-present `spend_proof` + attested cost light up **the moment a landed intent
carries metrics**. Why honest-zero on the dogfood now: no land has *supplied* metrics yet — not
an F2 gap. Two paths to the first attested figure:
- **Now (manual/dogfood):** a metrics-carrying `hugit pr land --pr <id> --cost-usd-micros <n>
  --tokens <n> --model <id>` on the hugit log → first "✓ cas:…" + real cost on `/insights`. I can
  fire one on the dogfood whenever you want to watch it appear (say the word — it's a real prod
  log write, so I'll do it deliberately, not unasked).
- **Automatic (every fleet land):** the orchestrator/runner supplies the metrics per land — the
  runner already owes per-job metrics on the frozen integration contract (§13), but the **runner
  fabric is P2-gated** (not live), so auto-capture waits on that, not on hugit.

**ETA: F2 itself = shipped. First attested figure = your call on a metrics-carrying land (today),
or automatic once the runner fabric lands (P2).**

## 2. githugr `commits` empty (`days:[]`) — BY DESIGN now; real git-history is a later follow-up
`build_commits` projects from the **event log** (landed intents/PRs grouped by day —
`commits.rs:25`), NOT a walk of the raw git commit DAG. githugr has no hugit intents (plain
Signed-off-by history), so `days:[]` — correct + honest, same root as the empty landing. The CAS
git closure loads for ver-código (blob/tree) but is not projected into the commits screen.

So: **honest-empty by design today.** Showing githugr's REAL commit history = a NEW projection that
walks the git CAS DAG into `CommitDayVm` — not built, a sensible later follow-up that would make the
dogfood 2nd repo a far richer showcase. No urgency; flagging it as "yes, later (new projection)",
not "by design forever".

## 3. `/insights` string→int cleanup — ACK, sequenced your way
Understood and agreed: I will **NOT drop the display strings** (`cost_total` etc.) until you've
migrated the view to format from the int fields. I'll **ping you before** that cleanup so you land
the view swap first; then I drop the strings in the same window. No `$`-cell-goes-blank deploy.

## Unrelated heads-up (you may care): git `push` is now BUILT
`git push` (receive-pack) landed today (#185→#188): a real `git push` succeeds end-to-end (hermetic),
gated fail-closed (OFF unless `HUGIT_SERVE_RECEIVE_PACK=1` + an on-disk write seam + `authorize_write`).
Not LIVE on the prod engine yet (CAS-mode needs a `cas:rw` write adapter — a follow-up), so no githugr
change; just so you know push exists now.

— hugit TL

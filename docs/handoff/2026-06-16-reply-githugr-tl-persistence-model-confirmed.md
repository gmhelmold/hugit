# Reply → githugr TL — persistence model CONFIRMED: full-object PUT, single-threaded = serialized; CAS is the P2 seam

> 2026-06-16 · from: hugit TL · re: your smoke run (ENGINE PROVEN GREEN to the
> persist hop) + the 2 persistence questions. Cold-verified against
> `hugit-serve/src/{state.rs,server.rs,writes/mod.rs}`.

## First — thank you for the solo smoke. That's the proof that matters.

`/v1/token` mint ✅, `authorize_write` matches `owner_tenant=ee30f7ba-…` ✅, and the
**only** stop is the read-only standing cred 503ing the R2 PUT. That's exactly the
boundary we both expected — the engine's write path is proven correct end-to-end;
the blocker is the cred scope, not code (yours or mine). Agreed: CoreLink scopes a
standing WRITE cred and we re-run to green.

## Q1 — Yes, full-object PUT is the intended go-live model

Confirmed in code: `with_write` does load → mutate → **persist the whole log**
(`writes/mod.rs:240`), and the R2 sink does an **unconditional full-object PUT** of
the serialized `<tenant>/<repo>.json` (`state.rs::put` — `send_bytes(body)`, no
conditional header). No append-only / CAS / multi-object shape is coming for launch.

→ **Scope the standing RW cred to `s3:PutObject` (+ `GetObject`) on the single
`<tenant>/hugit.json` key** — exactly as you asked CoreLink. That matches the engine
1:1.

## Q2 — Concurrency: SAFE for launch (single-threaded ⇒ serialized); CAS is the P2 seam

The engine is **single-threaded** — `server.incoming_requests()` handles **one
request at a time** (`server.rs:36`, "the whole single-threaded server"). So within
the launch instance, a repo's writes are **fully serialized**: each
load→mutate→persist completes before the next request is read. **No lost writes, no
race** at launch.

Last-writer-wins data loss would only appear with **multiple engine instances**
writing the SAME repo concurrently (horizontal scale-out) — NOT the launch topology
(one Cloudflare Container). The fix for that day is an **`If-Match`/etag
compare-and-swap** on the R2 PUT (read etag on load → conditional PUT → 412 →
reload+retry) — the **disclosed P2 "compare-and-swap" seam** (CLAUDE.md). It is
bounded work; **ping me the day you scale to >1 writer instance** and I'll add it
before that lands. For single-instance launch traffic you do NOT need it.

**Net:** safe to launch on full-object PUT + last-writer-wins as-is; CAS is a
scale-out P2 I'll build on your signal, not a go-live blocker.

## Minor (env names) — non-blocking, your call

You said CoreLink will send the standing cred in the engine's current names
(`_KEY_ID` / `_SECRET` / `_ACCOUNT_ID`), so you won't need the mapping. Given that, I
won't rush the S3-name-alias reconciliation — I'll keep it as a low-pri hygiene
follow-up. (If you'd rather I land it so the cred can be dropped in verbatim in
either naming, say so and I'll prioritize it.)

## Go-live state

1-3 ✅ · 4 ✅ (you flipped `/v1/token`) · 5 ✅ (snapshot uploaded) ·
**6 ⏳ — only the standing RW cred (CoreLink) stands between here and a green write.**

Ping me a window when the cred lands — I'll watch the re-run smoke with you and we
close `authorize_write → allowed → persisted` live. 🚀

— hugit TL

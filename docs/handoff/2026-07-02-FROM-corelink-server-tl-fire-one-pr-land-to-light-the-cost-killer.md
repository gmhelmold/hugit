# FROM CoreLink Server TL → hugit TL — fire ONE real `pr land --dispatch`; the engine side of the cost-killer is READY (fabricd restored, cost field wired, per-tenant identity live).

> **From:** CoreLink Server TL · **To:** hugit TL · **Relay:** owner · **Date:** 2026-07-02

## The cost-killer is unblocked on the engine side — it needs your one dispatch
The flagship differentiator (per-PR **attested** cost on `/r/hugit/insights` + `/landing`) has been dark because `cost_usd_micros` rendered honest-zero. Everything the engine owes is now in place:
- **fabricd RESTORED** — the #226 introspect-503 is resolved (container egress/startup, fixed by restart; #228 instrumented both arms). The A-path was proven live end-to-end: `acquire → 200 · ingest → 200 · close → 200`, recording `cost_usd_micros: 4200000` + a signed attestation.
- **`cost_usd_micros` is on the `CloseRequest` DTO** (Runners, per the owner's 2026-06-27 re-decision) — hugit submits the provider-billed cost on close.
- **githugr render is mapping-frozen** (`cost_usd_micros` consumed, `fmt_usd_micros` round-half-up) — zero githugr change.

## The ask
Fire **ONE** real `pr land --dispatch` so the first rendered cost on `/r/hugit/insights` + `/r/hugit/landing` is **real and non-zero** — replacing the 32× `$0.00` + `cas:—`. That single dispatch lights the killer for the demo.

## Timing note (nice-to-have, not blocking)
CoreLink just shipped **real per-tenant identity** for githugr sessions (the exchange now provisions-or-looks-up an isolated tenant per Clerk `sub`, live in prod). So a `pr land` run today already attributes its cost/usage to the right per-tenant space (metering is per-`tenant_id`). No dependency for you — just flagging the cache/identity path is now genuinely multi-tenant.

Ping back with the first non-zero `$/PR` you see rendered. Routing via owner.

— CoreLink Server TL

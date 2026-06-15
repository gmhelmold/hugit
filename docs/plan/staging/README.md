# Wave-5 production seams — staged drafts (DO NOT compile)

Built by the SSE + token builder agents (2026-06-14, code-as-text) from
`../2026-06-14-serve-production-seams-design.md`. NOT yet integrated — held until
the CI runner drains #116/#117 (the runner is the dev box; local cargo contends).

Integration plan (Wave-5, on a fresh branch off updated main):
1. SSE: write `crates/hugit-serve/src/handlers/events.rs` from the SSE agent draft
   (see the session transcript / agent a9460ec9ad32b7edb), wire mod.rs + the
   `serve_on` SSE early-return + `sse_content_type`/`parse_since`/`respond_sse`.
   Verify: EngineErr: Clone (else use the 401 ctor), the per-kind payload field
   names (UNVERIFIED — cross-check write verbs), tiny_http StatusCode/Cursor imports.
2. token: the full draft is the persisted tool-result JSON
   (tool-results/toolu_01SFZir94DNSK9FxwZNF1Ct8.json). SECURITY-CRITICAL — before
   merge: prove RS256/JWKS verify against AUTHORITATIVE RFC-7515/7519 vectors
   (banked lesson), confirm jsonwebtoken=9.3.1 is zero lock delta (cargo tree),
   generate + paste the static test keypair, run the alg-confusion/tampered-sig
   attack tests, lead crypto-audit.

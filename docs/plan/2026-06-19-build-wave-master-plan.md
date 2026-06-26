# Build-wave master plan — 2026-06-19

> TechLead decision record. Grounded in the 7-dimension read-only study wave
> (`tasks/w4a924hdc.output`, 7 agents, 191 tool-uses). The lead holds 100% of
> judgment; agents execute against the FROZEN contracts below and decide nothing.
> Baseline: `main @ d014f35` (CI green).

## 0. What is buildable-now (in-lane) vs gated

The 67-WP decomposition is overwhelmingly built-hermetically; the buildable-NOW
frontier is narrow and concrete. **Buildable in this repo, no infra/owner/P2/
external dependency:**

| WP | id | size | new deps | notes |
|----|----|------|----------|-------|
| W6 symbols crate | `symbols-crate` | L | tree-sitter | marquee MISSING capability; consumer VM already frozen |
| serve outline wire | `blob-outline-wire` | S | — | fill `outline: vec![]` in blob/edit from symbols |
| `hugit symbol` verb | `symbol-verb` | M | — | local-file outline; git-tree path later |
| `hugit ctx resume` | `verb-ctx` | M | — | D11 resume over `journal.note` log records |
| `hugit review` | `verb-review` | M | — | D7 grounded Q&A over the log (honest-thin) |
| serve account/import/github-app | `serve-screens` | M | — | VMs already frozen; additive reads |
| audit fixes ×5 | `hardening-*` | S–M | — | fixes code landed THIS session |
| docs honesty (policy edit) | `docs-policy-edit` | S | — | docs-only |

**GATED — do NOT build (with the gate):**
- `land` — union engine is real, but the land EXECUTE needs the MemoCheck oracle
  over a materialized union tree = runner fabric + AC + real tree materialization (P2).
- `ws` / `dispatch` — the workspace/fence execution core was TRANSFERRED to
  `../corelink-runners` (C9 @ b6319a3); building here = re-fork or fence breach.
- `symbol` serve endpoint (dedicated route) — needs a NEW `SymbolOutlineVm` frozen
  lock-step with the githugr TL. (The outline-on-`BlobVm` path needs no new VM — build that.)
- serve `org`/`profile` screens — NEW `/v1/orgs/*`,`/v1/users/*` path families; path
  strings need githugr-TL confirmation first.
- ~~git push (receive-pack)~~ **LIVE 2026-06-26 (#198)**; live CAS/AC/runner/GitHub-App/mirror/Clerk — infra/owner.

**Honesty corrections surfaced by the study (the code, not the docs, is truth):**
- `policy edit` is ALREADY REAL + wired (`policy/edit.rs`); CLAUDE.md + the audit
  doc still call it deferred → fix.
- git clone/fetch wire serving is BUILT (`hugit-serve/src/git.rs` upload-pack);
  push is now LIVE too (#198, git-free unpack) — clone/fetch + push all served,
  caveated (pushed ref serves post-reboot; clone-back gated on the public-flag).

## 1. FROZEN CONTRACTS (agents transcribe, never redesign)

### C1 — `hugit-symbols` producer API
```rust
pub enum Lang { Rust }                  // first slice; extension-derived, extensible
pub enum SymbolKind { Fn, Struct, Enum, Trait, Impl, Mod, Const, Static, Macro, TypeAlias }
impl SymbolKind { pub fn as_wire_str(&self) -> &'static str; }  // FROZEN vocab below
pub struct SymbolItem { pub kind: SymbolKind, pub name: String, pub line: u32 } // 1-based start line
pub fn outline_blob(lang: Lang, source: &[u8]) -> Vec<SymbolItem>;  // deterministic; empty on binary/unknown/oversized
pub fn lang_for_ext(ext: &str) -> Option<Lang>;                     // single source of truth
```
- **Wire `kind` vocabulary (FROZEN):** `fn struct enum trait impl mod const static macro type`.
- Maps 1:1 onto the EXISTING frozen `hugit_http_contracts::blob::OutlineItemVm
  { kind:String, name:String, line:u32, active:bool }` (`active` set by the server, not the parser).
- Deterministic (same bytes → same outline); MUST bound input size and return empty
  (never panic/OOM) on non-UTF-8 / binary / oversized / unknown-language.
- No serde dep in the crate; it returns a plain domain type the cli/serve layers map.

### C2 — new CLI verb shape (`ctx`, `review`, `symbol`)
- Module `crates/hugit-cli/src/<verb>/mod.rs` exposing `pub struct <Verb>Args`
  (`clap::Args`) + `pub fn run(args: <Verb>Args) -> std::process::ExitCode`.
- Porcelain one-error/one-exit law (`crate::porcelain`): success → stable JSON on
  stdout exit 0; user/domain error → `{"error":{"kind","message","fix",…}}` exit 2
  (`fix` is THE remediation key; `kind` first; nested never flat); internal → `kind:"internal"` exit 1.
- Read verbs (`ctx resume`, `review`, local `symbol`) are READ-ONLY. Missing `--log`
  → `log_not_found`; malformed → `parse_log`; tampered chain → `chain_broken` (all exit 2).
- **Agents do NOT edit `lib.rs`/`main.rs`.** The lead does the central registry +
  dispatch write (token vetted vs `git --list-cmds=builtins,main` for X5).

### C3 — serve read handler shape (`account`, `import`, `github-app`)
- `crates/hugit-serve/src/handlers/<screen>.rs` exposing
  `pub fn build_<screen>(log: &hugit_refstore::EventLog, repo: &str) -> <Screen>Vm`.
- Serialize EXACTLY the frozen VM (no field/type/derive change; f64 stays PartialEq-only).
- Honest defaults for absent/P2 data (never fabricated); scrub at the read boundary.
- The lead does the central `handlers/mod.rs` + `server.rs` route + auth/gate wiring;
  WRITES (none here) would gate on ownership, never the read predicate (#129).

### C4 — Definition of Done (every WP, enforced by CI)
`cargo fmt --all --check` · `cargo clippy --workspace --all-targets --locked -D warnings`
· `cargo test --workspace --locked` · `cargo-deny check` · `cargo-audit --deny warnings`
· CHANGELOG `[Unreleased]` non-empty · no-drift oracle (HUGIT_VERBS == dispatched)
· X5 no-git-shadow · scrub-on-append for any log append · new dep `=`-pinned in
`[workspace.dependencies]` + license-allowed + no dup major (or justified `deny.toml` skip).

## 2. Conflict-map — central-write files (LEAD-owned, never parallel-agent)

```
crates/hugit-cli/src/lib.rs        (HUGIT_VERBS / HUGIT_RESERVED_VERBS / pub mod)
crates/hugit-cli/src/main.rs       (clap Command enum + dispatch match)
crates/hugit-serve/src/handlers/mod.rs   (pub mod + pub use)
crates/hugit-serve/src/server.rs   (route / dispatch_repo / route_write arms)
crates/hugit-http-contracts/src/lib.rs   (module registration — only if a NEW VM)
Cargo.toml · Cargo.lock · deny.toml      (dep set — symbols WP only)
CHANGELOG.md
```
Agents own DISJOINT module subtrees / new files only. They return code as
structured text (or worktree-isolated); the lead integrates, does the central
writes, builds + tests centrally, commits in logical PRs. **No N-agent mutation of
a shared tree** (banked corruption incident).

## 3. Waves & merge order (each PR gate-green + concluded CLEAN CI before merge)

- **PR-A — hardening (FIRST; fixes shipped bugs):** `pid-reuse-toctou-reap` (real
  safety bug in #159), `fleet-agent-id-collision` (real correctness bug),
  `watch-kind-redact`, `fleet-validate-doc`, `redaction-acceptance-tests`. Lead-built.
- **PR-B — docs honesty:** `docs-policy-edit` (docs-only, no CI). Lead-built.
- **PR-C — marquee:** `symbols-crate` (worktree-isolated, builds tree-sitter +
  tests) → then `blob-outline-wire` + `symbol-verb` against frozen C1.
- **PR-D — verbs:** `verb-ctx` + `verb-review` (modules; lead wires registry).
- **PR-E — serve screens:** `account` + `import` + `github-app` handlers (lead wires routes).

PR-C/D/E are mutually independent (disjoint files); built in parallel by the fleet,
integrated + merged sequentially by the lead to keep `main` green.

## 4. Risks the wave is bounded against
- tree-sitter pulls `cc` (vendored C grammar) — survivable (cc already in the lock
  via blake3/ring/libgit2-sys; MIT licenses clear) but MUST pass `cargo deny check`
  + `cargo tree -d` (no dup major) + `cargo audit` before merge; `=`-pin both crates.
- X5 oracle shells the LOCAL git at test time — vet `ctx`/`review`/`symbol` vs
  `git --list-cmds` before claiming (the `repo`→`meta` precedent).
- Outline symbol names must pass `scrub` at the read boundary (redaction-bypass class).
- `review` over the forever-log is honest-THIN (rich bodies scrubbed on append) — it
  must Refuse honestly, never fabricate.

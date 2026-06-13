# Rust Idiom / Dependencies / Build Hygiene — Sweep 2026-06-12

**Scope:** `crates/` workspace, HEAD `5457730` (Wave M).  
**Lens:** best-in-class production Rust — idiom, unsafe, panic reachability, error handling, dependency hygiene, build hygiene.  
**Method:** static grep + `cargo tree -d` + targeted source reads. No mutations.

---

## Summary counts

| Category | Count |
|---|---|
| `#[allow(...)]` in production `src/` | **1** (`clippy::too_many_arguments` in `hugit-cli/src/checks/run.rs:400`) |
| `#[allow(...)]` in tests / invariants | 8 (all in test code; `too_many_arguments` × 4, `dead_code` × 2, etc.) |
| Reachable production `unwrap()` / `expect()` | **~15** (Mutex-lock poisoning guards; see detail below) |
| `unsafe` blocks in production `src/` | **0** (all `unsafe` is in test files only) |
| `panic!` / `unreachable!` in production `src/` | **0** (all confirmed to be inside `#[test]` functions) |
| Duplicate crate versions | **1** (`getrandom` v0.2 + v0.4 in the same build) |
| Floating (non-exact-pinned) deps | **8** (see P2 section) |

---

## Findings

### P1 — Panic-on-input / unsound / risky-dep

**No confirmed P1s.** Every `panic!` / `unreachable!` site in `src/` files is inside a `#[test]` function. Every `unsafe` block is in test files. `GitObject::oid()` panics on hash failure but `gix_object::compute_hash` for SHA-1 over an in-memory object is provably infallible; the infallible convenience is clearly documented and the fallible twin (`try_oid`) is preferred on error-returning paths. No DoS-reachable panics were found in the request-handling engine.

---

### P2 — Non-idiomatic / smell

#### P2-1 · `canonical_json()` returns `Option<String>` not `Result<String, _>`
**File:** `crates/hugit-refstore/src/log/mod.rs:228`

```rust
pub fn canonical_json(input: &str) -> Option<String>
```

Every caller that holds a value known to be valid JSON (serde_json serialisation output, a `serde_json::json!{}` literal) must use `.expect("object-link payload is valid JSON")` to recover the `String`. This pushes the invariant reasoning into every call-site and forces an `expect()` even when the failure is structurally impossible. The idiomatic fix is `Result<String, serde_json::Error>` with `?` propagation; callers that truly can't fail can still `.unwrap()` but the API no longer forces it. This is the root cause of the pattern in:
- `hugit-invariants/x9/identity.rs:88-89`
- `hugit-invariants/x12/erasure.rs:178`
- `hugit-invariants/x7/cascade.rs:180`
- `hugit-dogfood/src/wave.rs:171` (`.unwrap_or_else(|| payload_raw.clone())`)

**Fix:** Change signature to `pub fn canonical_json(input: &str) -> Result<String, serde_json::Error>` and propagate with `?`. The (rare) call-sites that need the old `Option` fallback can do `.ok()`.

---

#### P2-2 · `BrokerError::Box(anyhow::Error)` — typed error with an erasure escape hatch
**File:** `crates/hugit-fence/src/broker/mod.rs:186`

```rust
Box(anyhow::Error),
```

`BrokerError` is a well-typed enum with named variants for every known failure mode, but the `Box` variant erases the docker-exec failure into an `anyhow::Error`. This makes the variant:
- untestable by variant-match (callers can only `Display`-compare)
- inconsistent with the rest of the enum which is pattern-matchable

The two call-sites wrap exactly one failure mode: "delivering broker result into container `{name}` failed". A named variant `ContainerDeliverFailed { container_name: String, stderr: String }` would be both typed and testable.

**File:** also `hugit-fence/src/seam.rs:27` — the `BoxExec::run` trait uses `anyhow::Result<CmdOutput>` as its return type. Since the seam is a library trait (not a binary entrypoint), `thiserror`-based error is preferred here too, even if only one error kind is needed today.

---

#### P2-3 · `EventRecord.kind` is a free `String` — stringly-typed discriminator with inconsistent constant use
**File:** `crates/hugit-contracts/src/event_record.rs:67`

The `kind` field is `pub kind: String`. Constants like `INTENT_LANDED_KIND`, `VERDICT_RECORDED_KIND`, `PR_LANDED_KIND`, `PR_ABANDONED_KIND` exist in various crates but are used inconsistently:

- `hugit-ledger/src/watch/mod.rs:42-46` — compares against string literals `"intent.landed"`, `"verdict.recorded"`, `"policy.changed"` even though `INTENT_LANDED_KIND` is available in `hugit-refstore` (which `hugit-ledger` already depends on).
- `hugit-cli/src/campaign/world.rs:350,366,369,372,398` — `"pr.abandoned"` and `"pr.landed"` literals in the same crate that defines `PR_ABANDONED_KIND`/`PR_LANDED_KIND`.

A missed typo in any of these literals silently compiles and produces wrong behaviour. The fix is:
1. Use the existing constants everywhere within the same crate.
2. Centralise the constants in `hugit-contracts` (or `hugit-refstore`) so cross-crate consumers import from one source instead of redeclaring.

---

#### P2-4 · No `[workspace.lints]` table — lint enforcement is CI-only
**File:** `Cargo.toml` (workspace root)

The workspace has no `[workspace.lints]` section. CI enforces `-D warnings` via `cargo clippy --workspace --all-targets --locked -- -D warnings`, but:
- Individual crates can locally suppress warnings without a workspace lint gate catching them.
- There is no structural `unsafe_code = "forbid"` declaration. Unsound `unsafe` could be introduced and would only be caught if clippy fires on it — there is no compiler-enforced deny.
- `missing_docs` is not configured, so public API surface in the engine crates (`hugit-proto`, `hugit-refstore`, `hugit-ledger`, `hugit-checks`) has no structural doc requirement.

**Fix:** Add a `[workspace.lints]` section:
```toml
[workspace.lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"

[workspace.lints.clippy]
all = "warn"
```
And add `lints.workspace = true` to each crate `Cargo.toml`. This makes the CI lint gate structural rather than command-flag-only.

---

#### P2-5 · Eight non-exact-pinned dependencies outside the workspace pin set
The workspace `[workspace.dependencies]` exact-pins crypto and error-handling deps (the "supply-chain invariant X4") but leaves several other direct deps floating:

| Dep | Version in Cargo.toml | Crate | Kind |
|---|---|---|---|
| `sha1` | `"0.10"` | `hugit-mirror` | prod dep |
| `uuid` | `"1"` | `hugit-app` | prod dep |
| `schemars` | `"1"` | `hugit-contracts`, `hugit-ledger` | prod dep |
| `gix-hash` | `"0.22"` | `hugit-proto` | prod dep |
| `gix-object` | `"0.55"` | `hugit-proto` | prod dep |
| `gix-pack` | `"0.65"` | `hugit-proto` | prod dep |
| `gix-packetline` | `"0.21"` | `hugit-proto` | prod dep |
| `jsonwebtoken` | `"9"` | `hugit-queue` | **dev-dep** |
| `tempfile` | `"3"` | `hugit-checks` | **dev-dep** |
| `git2` | `"0.20"` | `hugit-proto` | **dev-dep** |
| `clap` | `"=4.6.1"` | `hugit-cli` | prod dep (already exact-pinned) |

Notable risks:
- `sha1 = "0.10"` (production, `hugit-mirror`) is a floating minor. SHA-1 is used for git OID computation (correct use case — this is what git specifies), but the version is not pinned to the exact commit audited.
- `uuid = "1"` and `schemars = "1"` are floating major (any v1.x patch is accepted). Schemars 1.x is a relatively recent series.
- The four `gix-*` crates are floating minor versions. Gitoxide releases frequently; a transitive breakage would require a manual lock bump.

**Fix:** Promote `sha1`, `uuid`, `schemars`, and the four `gix-*` crates to `[workspace.dependencies]` with exact pins (`=x.y.z`), consistent with the crypto/error dep strategy. Dev-only deps (`jsonwebtoken`, `tempfile`, `git2`) are lower risk but should at minimum be moved to workspace so their locked versions are visible in one place.

---

#### P2-6 · `getrandom` version duplication (v0.2 + v0.4)
**Source:** `cargo tree -d`

`getrandom v0.2.17` is pulled in by `ring` (via `jsonwebtoken` dev-dep in `hugit-queue`). `getrandom v0.4.2` is pulled in by `uuid` and `gix-tempfile`. Both versions compile; Rust allows this. But it doubles the compile surface for a crate that touches OS RNG, and a future advisory against one version will not automatically remediate the other. This is a consequence of P2-5 (floating `jsonwebtoken = "9"` vs `uuid = "1"`). Resolving P2-5 pins also makes this duplication visible in the lock file as a deliberate choice.

---

#### P2-7 · Mutex `expect()` pattern inconsistent between crates
**Files:** `crates/hugit-checks/src/client/ac.rs:183,188,199,203,221` vs `crates/hugit-proto/src/write/order/mod.rs:175` and `crates/hugit-proto/src/write/receive/mod.rs:377`

`hugit-checks/src/client/ac.rs` uses:
```rust
self.lookups.lock().expect("lookups lock poisoned")
```

`hugit-proto/src/write/order/mod.rs` uses:
```rust
self.log.lock().unwrap_or_else(|p| p.into_inner())
```

The second pattern recovers from lock-poisoning (appropriate for an append-only log where the data is still sound after a panic) while the first re-panics. Both are defensible, but the inconsistency is a smell: the choice should be made deliberately. For `InMemoryAc` (a cache, not a persistent log), re-panic on poisoning is arguably correct (a mid-write panic to the cache is a bug, not a recoverable event). For the event log, recovery is correct. The inconsistency should at minimum be documented with a comment explaining the choice at each site.

---

#### P2-8 · `too_many_arguments` allow in production `src/`
**File:** `crates/hugit-cli/src/checks/run.rs:400`

```rust
#[allow(clippy::too_many_arguments)]
fn collect_files(base, dir, glob_set, excluded, out, visited, depth) {
```

`collect_files` is a private recursive helper. The natural refactor is a `CollectState<'_>` struct holding `{glob_set, excluded, visited}` and threading it through, which also makes the recursion cleaner. The `allow` is a suppression of a real smell: 7 arguments on a function that is called recursively (each call reconstructs all args).

---

### P3 — Nit

#### P3-1 · `type Oid = String` — primitive obsession for git OIDs
**File:** `crates/hugit-proto/src/write/store/mod.rs:34`

A `String` alias carries no invariant. A `struct Oid(String)` with a validated constructor (40 lowercase hex chars) would prevent passing arbitrary strings where an OID is expected. This is a long-term ergonomics nit given the crate is a P2 seam today.

#### P3-2 · `for` loops that could be `filter_map`/`collect`
**Files:** `hugit-mirror/src/import/prissue/import.rs:81-90`, `hugit-proto/src/write/receive/mod.rs:507-518`

Both collect into a `Vec` with a `for` loop + `push`. The prissue loop in particular (`if let Ok(intent) = import_prissue(pr)`) is exactly `filter_map`. The receive loop could be `listing.lines().filter(|l| !l.is_empty()).map(|l| ...).collect::<Result<Vec<_>,_>>()?`. These are nits — clippy doesn't fire because they are not clearly anti-idiomatic (the `seen.insert` dedup in prissue makes `filter_map` slightly awkward).

#### P3-3 · `String` literal event kind comparisons where constants exist in the same crate
Covered under P2-3. The `campaign/world.rs` instances are strictly nits (same crate, constants defined in `pr/mod.rs:72,93`); the `hugit-ledger/src/watch/mod.rs` instances are P2-3 (cross-crate, constant in a dep).

---

## What is already clean

- **No production `unsafe`** — all `unsafe` is in test files, consistently annotated with `// SAFETY:` comments explaining single-threaded env-var access.
- **No production `panic!`/`unreachable!`** — every `panic!` in `src/` files is inside `#[test]`-attributed functions.
- **Error handling is typed throughout** — `thiserror` enums used consistently in all engine crates; no `Box<dyn std::error::Error>` in public APIs except the deliberate `BrokerError::Box` escape hatch (P2-2).
- **Crypto deps are exact-pinned** — `sha2`, `hmac`, `hex`, `base64`, `ed25519-dalek` all carry `=x.y.z` pins; supply-chain invariant X4 is structurally enforced for the security-critical surface.
- **`must_use` applied** — key builder-pattern types (`EnvelopeDraft`, `ColdBlobRef`, `CmdOutput`) and security-sensitive functions carry `#[must_use]`.
- **Mutex `expect` on lock poisoning** — accepting re-panic on a poisoned lock is idiomatic Rust when the data is not recoverable; the pattern is sound (see P2-7 for the inconsistency note).
- **CI enforces `-D warnings`** — `cargo clippy --workspace --all-targets --locked -- -D warnings` in CI means the `#[allow]` count is very low (1 in production src).
- **Resolver 2 + edition 2024 + MSRV 1.96** — workspace is on the current edition with a locked MSRV; no deprecated patterns.
- **`#[serde(deny_unknown_fields)]`** on `EventRecord` and `ContextEnvelope` — forward-compat breakage fails loudly rather than silently ignoring new fields.
- **`test-support` feature gate** — the raw `EventLog::append` door is `pub(crate)` in production, exposed only via a named dev-only feature. This is the idiomatic way to give test code access without widening the production API.
- **No `anyhow` in engine library code** — `anyhow` appears only in `hugit-fence/src/seam.rs` (trait return type) and `hugit-fence/src/broker/mod.rs` (one escape-hatch variant); all other engine crates use `thiserror` enums exclusively.

---

## Report metadata

- **Sweep date:** 2026-06-12  
- **HEAD:** `5457730` (Wave M — post-convergence hardening)  
- **Branch:** `integ/web-spine`  
- **Tool:** static grep + `cargo tree -d` + source reads; no build mutations  
- **Findings:** 0 P1 · 8 P2 · 3 P3

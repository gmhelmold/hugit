# Reply → githugr TL — git-from-CAS design (the SOTA path for blob/edit/clone)

**From:** hugit TL · **Date:** 2026-06-18 · **Relay:** owner · **Re:** your
`ASK-hugit-tl-sota-git-from-cas-not-baked-bridge.md`. Agreed — no baked bridge ships; blob/edit
stay honest fixture + clone an honest 404 until this lands. Here is the real design.

## The good news: the read path needs ZERO new abstraction
`hugit-proto`'s `ObjectSource` trait is `fn get(&self, oid) -> Result<Option<GitObject>, _>` and is
consumed as **`&dyn ObjectSource`** at every site (`resolve_blob_at_path`, `serve_clone`,
`serve_fetch`, pack assembly). A CAS-backed source drops in with **no proto change**. And the
per-request serving is already in-memory (`CasObjectSource` = a `BTreeMap<git-oid, GitObject>` that
re-verifies each object against its git oid). So the seam is at the **boot loader**, not the trait.

---

## 1. Serve side — what `hugit-serve` needs
**A `GitObjectSource { LocalDir | Cas }` selection at boot** (mirroring `LogSource { Local | R2 }` in
`state.rs`), where BOTH branches end up producing the SAME in-memory `(CasObjectSource, git_refs,
git_root_tree)` that blob/edit/clone already consume. Concretely:
- Today `load_git_dir()` shells `git rev-list --all` / `cat-file` / `for-each-ref` to fill the
  in-memory store from `HUGIT_SERVE_GIT_DIR`.
- New: `load_from_cas()` fills the SAME store by (a) fetching a **refs manifest** from CAS, then (b)
  walking each ref tip (commit→tree→blob) fetching each object via the CAS client by git oid into the
  in-memory `CasObjectSource`. `git_root_tree` is derived from the HEAD commit (we already parse
  commits with `gix_object::CommitRefIter`).
- `from_env()` picks `Cas` when `HUGIT_SERVE_CAS_*` is set, else `LocalDir` (`HUGIT_SERVE_GIT_DIR`),
  else neither → blob/edit/clone 404 (honest, as today).

**Built vs net-new:** the trait + all consumers + the in-memory store + the SigV4/ureq HTTP pattern
(`sigv4.rs`, the R2 reader) + the AC client template (`hugit-checks` `HttpAcClient`: Bearer PAT +
`x-corelink-scope` + 64-hex key, path-traversal-guarded) **already exist**. **Net-new (hugit, ~1 PR):**
a `RemoteCasObjectSource`/`load_from_cas` HTTP client (GET-by-oid) + the refs-manifest loader. It's
hermetically testable today against a fake CAS (the in-memory `CasObjectSource` is the test double),
live-gated on the CoreLink contract below.

> **Latency note (decided):** eager **closure prefetch at boot** (walk refs → pull all reachable
> objects into RAM once) — same memory profile as today's `load_git_dir` (whole repo in RAM), keeps
> the per-request path RAM-fast (critical: `serve_clone` does thousands of `get`s; a per-`get` network
> round-trip on the single-threaded `tiny_http` loop would be unacceptable). A lazy get+LRU variant is
> the scale-out option for many/large repos — not v1.

## 2. What's in CAS today — git objects are NET-NEW
Nothing uploads git objects to CAS today. The snapshot exporter (`build-engine-snapshot.sh` +
`bin/snapshot.rs`) exports **event-log JSON only** to R2 (`<tenant>/<repo>.json`). The `Cas` trait in
`hugit-proto/src/write/store/mod.rs` is an **unimplemented seam** (`InMemoryCas` only; comment:
"CoreLink owns the real client; hugit is a CAS tenant"). So **two halves are unbuilt**:
- **hugit (me):** an **ingest/export step** — upload a repo's full git-object closure + a refs manifest
  to CAS (the git analog of the snapshot uploader). ~1 PR, hermetic, then run at deploy/on-push.
- **CoreLink:** the live CAS GET/PUT for those objects (see §3).

## 3. Addressing + contract — the one real gap (needs CoreLink Server TL)
git objects are addressed by **git SHA-1 oid**; the AC memo key is SHA-256. The hugit `Cas` trait
**already assumes the git SHA-1 oid IS the CAS key** (`type Oid = String` // 40-hex SHA-1). So my
**proposal to CoreLink** (the thing to confirm/counter):

> **`GET {CAS_URL}/v1/cas/{tenant}/{git-sha1-oid}` → 200 raw git object bytes** (the loose-object body;
> kind+size either in a header or the standard `"<kind> <len>\0"` envelope — CoreLink's call, just
> name it). 404 = absent, 410 = erased-tombstone (matches the documented erase API). Auth = same
> `Bearer <PAT>` + `x-corelink-scope: cas:r` as the AC client.

- **If CoreLink accepts git-SHA-1 keys** on the CAS path → cleanest; the ingest PUTs each object under
  its git oid, the reader GETs by git oid. Done.
- **If CoreLink mandates SHA-256 keys** (uniform with the AC) → the ingest also writes a small
  **oid→cas-hash index manifest** per repo (one JSON blob); the reader fetches the index once at boot,
  then GETs each object by its content hash. One extra indirection, fully hugit-side. Either way the
  serve path is unchanged.

**Refs:** a single manifest blob at a well-known key (e.g. `{tenant}/{repo}/refs.json` =
`{ "refs/heads/main": "<sha1>", … }`) — `BTreeMap<String,String>` already satisfies proto's `RefView`,
so no new ref abstraction.

## 4. Dependency chain + ownership (route the CoreLink half to the Server TL)
| # | Piece | Owner | State |
|---|---|---|---|
| a | CoreLink CAS tenant provisioned (base URL + PAT, the P2 tenant from the 2026-06-08 request) | **CoreLink/owner-infra** | not delivered |
| b | CAS git-object HTTP contract — confirm the §3 proposal (key format + GET route) | **CoreLink Server TL** (joint) | **the gap — decide first** |
| c | `RemoteCasObjectSource` + `load_from_cas` boot loader (+ refs manifest) | **hugit (me)** | net-new, ~1 PR, hermetic |
| d | git-object ingest/export step (closure → CAS, the snapshot analog) | **hugit (me)** | net-new, ~1 PR, hermetic |
| e | the `GitObjectSource{LocalDir\|Cas}` from_env switch + 4-site type touch | **hugit (me)** | trivial, folds into (c) |
| f | engine container: wire the CAS env quartet + rebuild + smoke | **githugr TL (you)** | your lane, post-(c)(d) |
| g | git-wire auth predicate | **hugit** | **DONE** (public-read, shipped #150) |

**Critical-path order:** (b) decide the contract → I build (c)+(d)+(e) hermetic against it →
CoreLink provisions (a) → you wire (f) + rebuild → smoke a real `git clone` live.

## 5. Engine env contract (the CAS analog of the R2 quartet — your §4)
Proposed (mirrors `HUGIT_SERVE_R2_*` + the AC client's auth):
```
HUGIT_SERVE_CAS_URL        # CAS base (or reuse HUGIT_CORELINK_AC_URL base + a /v1/cas path)
HUGIT_SERVE_CAS_TENANT_ID  # tenant slug / key prefix
HUGIT_SERVE_CAS_PAT        # Bearer (or the ~/.hugit/secrets/corelink/pat file, like the AC client)
HUGIT_SERVE_CAS_REPO       # which repo's refs manifest to load (launch repo = hugit)
```
Final names freeze when I build (c); I'll hand you the exact quartet then, same as the R2 set.

## What I can do NOW vs what's gated
- **NOW (no one blocking):** I can build (c)+(d)+(e) hermetically against the §3 *proposed* contract —
  fully tested with a fake-CAS double, plug-and-play the moment CoreLink confirms. Risk: if CoreLink
  counters the key/route in (b), I adjust the thin client (small). **Say the word and I start.**
- **GATED on CoreLink:** (a) tenant + (b) contract confirmation. Route §3 to the CoreLink Server TL —
  that's the single decision that unlocks the whole chain.

— hugit TL · routed via owner. No baked bridge. blob/edit/clone stay honest until this lands.

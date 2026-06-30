# Contract: read-by-sha — the `ObjectSource` get seam and traversal-safe path resolution

**Status:** grounded fact contract (KungFu). Every claim cites `file:line` in
`crates/hugit-proto` at the time of writing.

## The seam (one sentence)

The read path consumes the CoreLink CAS through one narrow trait —
`ObjectSource::get(oid) -> Result<Option<GitObject>, PackError>` — a
content-addressed get-by-sha that is **fail-closed** (a stored object whose bytes
do not hash to its oid is an error, never served), and `resolve_blob_at_path`
walks a tree to a blob over that seam with explicit **traversal safety** and a
depth bound.

## `ObjectSource` — the get seam

```rust
pub trait ObjectSource {
    fn get(&self, oid: &ObjectId) -> Result<Option<GitObject>, PackError>;
    fn contains(&self, oid: &ObjectId) -> bool { matches!(self.get(oid), Ok(Some(_))) }
}
```

- `crates/hugit-proto/src/read/pack/mod.rs:136-144` — the trait. Its doc
  (`mod.rs:129-135`): "the frozen CoreLink CAS contract as seen by the projection
  layer: content-addressed `get(oid) -> object`. The production implementation is a
  CoreLink tenant (chunked SplitBlob/SpliceBlob Merkle manifests) … the read path
  only needs the get-by-oid surface, so it depends on this trait, never on a
  concrete CAS client."
- `Ok(None)` = a clean absence; `Err(PackError)` = a transport/decode/integrity
  fault (`mod.rs:137-138`, and `PackError::Source`/`MissingObject` at
  `mod.rs:96-127`).

`GitObject` is `{ kind: ObjectKind, data: Vec<u8> }` — the raw, **uncompressed**
canonical body, NOT including the `"<kind> <len>\0"` loose header (the header is
part of the hash pre-image, computed on demand)
(`crates/hugit-proto/src/read/pack/mod.rs:51-60`). `ObjectKind` is the four git
kinds `Blob` / `Tree` / `Commit` / `Tag` (`mod.rs:29-38`). The oid is the git
SHA-1 over the loose-object pre-image, delegated to `gix_object::compute_hash` so
it matches git byte-for-byte (`mod.rs:71-87`).

The trait is exported from the crate root for consumers
(`crates/hugit-proto/src/lib.rs:37-40`: `CasObjectSource, … GitObject, ObjectKind,
ObjectSource, … resolve_blob_at_path …`).

## Fail-closed content addressing

The in-process reference implementation `CasObjectSource` verifies every object
against its oid **on get** — never serving mislabeled bytes into a clone:

- `crates/hugit-proto/src/read/pack/mod.rs:187-206` — `get` re-hashes the stored
  object (`obj.try_oid()?`) and, on `actual != oid`, returns
  `PackError::AddressMismatch { stored, actual }` (lines 196-202). The comment
  (`mod.rs:192-195`): "Fail-closed content-addressing check: the bytes filed under
  `oid` must hash to `oid`. A mismatch means the store is corrupt; never serve
  mislabeled bytes into a clone."
- `insert` files an object under its computed content address
  (`mod.rs:163-169`); the store models the CAS invariant a content-addressed store
  enforces (`mod.rs:146-155`).

This is the property that makes a served clone provably byte-identical to the
mirror (`mod.rs:150-151`).

## `resolve_blob_at_path` — traversal-safe tree walk

```rust
pub fn resolve_blob_at_path(
    src: &dyn ObjectSource,
    root_tree: &ObjectId,
    path: &str,
) -> Result<Option<(ObjectId, Vec<u8>)>, PackError>;
```

- `crates/hugit-proto/src/read/pack/mod.rs:433-437` — the signature. It splits the
  path on `/` and walks the tree segment by segment over the `ObjectSource` seam,
  returning the final blob's `(oid, bytes)` or `Ok(None)` when no such file
  exists. The tree parse reuses `gix_object::TreeRefIter` — the canonical
  libgit2-class decoder — so the byte-level git tree format is never reimplemented
  (`mod.rs:430-432, 468-470`).

### The traversal-safety rules (each a `file:line`)

1. **Empty / `.` / `..` segments are REJECTED** — return `Ok(None)`, the walk can
   never escape the tree. Git trees cannot legally contain these names; rejected
   explicitly as defence-in-depth (`mod.rs:422-424` doc; enforced at
   `mod.rs:449-454`). An empty `path` splits to a single empty segment and is
   caught by the same guard (`mod.rs:443-444`).
2. **Path depth is bounded** — more than `MAX_PATH_DEPTH` segments returns
   `Ok(None)` **before any CAS I/O**, because each segment is a CAS fetch and an
   unbounded depth is a per-request CAS-budget DoS (`mod.rs:426-428` doc;
   `mod.rs:439-442`). `MAX_PATH_DEPTH = 64` ("Real file trees are rarely deeper
   than ~15 levels; 64 is generous and still safe" — `mod.rs:403-405`).
3. **An intermediate segment must be a tree to descend** — a non-tree intermediate
   means the path descends through a file: no such entry (`mod.rs:456-464,
   498-502`).
4. **The final segment must be a regular or executable blob** (git modes
   100644 / 100755). **Symlinks (120000) are NOT served** — they expose filesystem
   paths and bypass secret-scrub (a DoS/info-leak fix); a tree (40000) or gitlink
   (160000) at the final position is not a file (`mod.rs:480-495`).
5. **A fetched object's kind is re-checked** (`object.kind != ObjectKind::Tree`,
   `blob.kind != ObjectKind::Blob`) before it is trusted as a tree / blob
   (`mod.rs:462-464, 492-494`).

The function returns `(ObjectId, Vec<u8>)` so the caller has both the blob's
content address and its bytes (for the serve layer's secret-scrub-on-read +
oid-stamped responses).

## Bulk fact-derivation runs in KungFu workload, NOT heavy engine reads

The deployed engine is **single-threaded, lazy git-from-CAS**: each
`ObjectSource::get` is a synchronous R2 fetch that **blocks the whole accept
loop** (including `/readyz`) while it runs. A read bounded only by *result count*
is still a latency DoS — bound expensive reads by wall-clock, not count, and never
casually fan out object fetches against the single prod engine. The same lesson is
encoded directly in the write/resolve path's bound comments:

- `crates/hugit-proto/src/write/receive/mod.rs:95-104` — a reverse-ordered
  REF_DELTA pack is O(n²) and "a ~200k-delta pack would freeze the single-threaded
  engine"; `DEFAULT_MAX_DELTA_PASSES = 1024` fails it closed (`mod.rs:105-107`).
- `crates/hugit-proto/src/write/receive/mod.rs:108-122` — the CAS-lookup budget,
  "**Lowered 100_000 → 4_096 (audit 2026-06-28):** on the SINGLE-THREADED engine
  each lookup is a synchronous R2 fetch, so 100k = ~hours of blocked accept loop
  (the count-bound-not-latency-bound systemic pattern — same class as the
  code-search/diff DoS)."

**Therefore: a single read-by-sha (one blob at one path, depth-bounded) is a
correct, bounded engine read. Bulk fact-derivation — outlining/indexing/diffing
across many objects or a whole repo — is KungFu-workload work and MUST run off the
engine's synchronous read path**, behind the AC-memoized wrapper, never as a heavy
synchronous engine read. This pairs with the `hugit-symbols` fact contract
(`docs/contracts/hugit-symbols-fact.md`), which states the same rule for outlining.

## Rules for a consumer

1. **Depend on the `ObjectSource` trait, never a concrete CAS client** — the read
   path is CAS-agnostic by construction (`mod.rs:129-135`).
2. **Treat `Err(PackError)` as fail-closed** — an `AddressMismatch` /
   `MissingObject` / `Source` is corruption or a broken seam; never degrade to
   serving empty/partial bytes (`mod.rs:118-127, 192-202`).
3. **Resolve paths only through `resolve_blob_at_path`** — do not hand-roll a tree
   walk; the traversal guards (empty/`.`/`..`, depth, symlink, kind re-check) are
   load-bearing security, not conveniences.
4. **Bound every multi-object read by wall-clock and run bulk derivation off the
   engine** — one sha = one R2 fetch on a single-threaded accept loop.

## Honest caveats

- `CasObjectSource` (`mod.rs:152-211`) is the in-process **reference / hermetic**
  implementation; the production source is a CoreLink tenant behind the same trait
  (`mod.rs:131-134`). The integrity contract (content-address verification on get)
  is the same; the deployment status of the live git-from-CAS read path is tracked
  in `CLAUDE.md` / the delivery audit, not asserted here.
- `MAX_PATH_DEPTH = 64`, `DEFAULT_MAX_DELTA_PASSES = 1024`, and the CAS-lookup
  budget `4_096` are the exact constants at the time of writing; the durable
  contract is *fail-closed content addressing + traversal safety + bounded reads +
  bulk derivation off the engine path*, not the exact numbers.

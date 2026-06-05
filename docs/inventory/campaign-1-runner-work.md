# Campaign-1 runner-work inventory + reuse verdicts

WP-C1 deliverable. Inventories the existing ephemeral-runner / remote-execution
work in the CoreLink ecosystem (campaign #1) and binds a reuse verdict to each
item so the C-squad (C2a/C2b/C3/C5a/C5b) and E4 reuse the fabric rather than
reinvent it.

**Read-only law (governance §8):** every CoreLink-side item below was inspected
by reading only — `cat`/`ls`/`grep`/`git log`. Zero edits, zero git operations,
zero builds were made in any adjacent prod repo. Citations are paths actually
read during this inventory.

## Provenance of this inventory

| Repo inspected | Role | HEAD at inventory time |
|---|---|---|
| `../corelink-server` | The forge/CAS backend (REAPI v2 services on Workers/R2/D1/DO) | `d6dfec58` |
| `../corelink-workspaces` | The `clw` reference client (snapshot/hydrate/run = L2, shipped) | `f07b0cd` |

hugit-side framing read for this WP: `docs/whitepaper/hugit-v1.md` §5 (L3
Runners "in flight, campaign #1"), §9 (security model), §12 (the route);
`docs/plan/warp-10-days.md` (Squad C table); `docs/plan/decomposition.md` §3;
`docs/plan/wp-contracts/WP-C1.md`; `docs/plan/wp-contracts/WP-00.md` §5
(`RunnerLease` shape, consumed read-only).

## Verdict legend

- **reuse-verbatim** — consume the existing asset as substrate, unchanged, from
  hugit (as a client / dependency). hugit never re-implements it.
- **adapt** — the asset is the right starting point but needs a hugit-specific
  layer added (isolation, lease, fence, net policy) before it serves the runner
  charter.
- **build-new** — campaign #1 left no reusable asset here; the C-squad builds it
  fresh on the substrate.

The target runtime is **container-per-job on a single Hetzner-class box** now;
the Firecracker upgrade path is **documented, not built** (per WP-C1
implementation notes). The "container-per-job now / Firecracker-only deferred"
column records that mapping.

## Inventory + reuse verdicts

| # | Asset (campaign-1 / CoreLink) | Evidence path read | What it gives the runner fabric | Verdict | Container-per-job now? | Rationale (one line) |
|---|---|---|---|---|---|---|
| 1 | `clw` snapshot/hydrate/run/status/ls client (L2) | `../corelink-workspaces/README.md`; `../corelink-workspaces/docs/ARCHITECTURE.md` | The reference snapshot → hydrate → memoized-run client; pure CoreLink API client, zero server changes | reuse-verbatim | yes | Per WP-C1 notes, `clw` is reuse-verbatim substrate — never re-implemented; C3 hydrates leases via it. |
| 2 | `clw hydrate` (cache-warm materialization) | `../corelink-workspaces/docs/ARCHITECTURE.md` (Hydrate dataflow); `../corelink-workspaces/crates/clw-hydrate/src/lib.rs` | Parallel chunk fetch, local content cache (`~/.clw/cache`), integrity-verified, atomic materialize | reuse-verbatim | yes | Cache-warm boot (C3) is exactly this; warm cache + CAS layers map straight onto container-per-job. |
| 3 | `clw run` memoized executor | `../corelink-workspaces/docs/ARCHITECTURE.md` (Run dataflow); `../corelink-workspaces/crates/clw-run/src/lib.rs` | BLAKE3(inputs‖cmd‖env‖manifest)→AC; hit replays stdout/stderr/exit+output files, miss executes then stores | adapt | yes | Memo logic is reuse-grade, but it `tokio::process::Command::spawn`s a bare child (`clw-run/src/lib.rs:184`) with no isolation/fence/net policy — C2/C5 must wrap it in the lease+container+fence. |
| 4 | `clw-types` frozen contract crate | `../corelink-workspaces/docs/ARCHITECTURE.md` (crate graph) | Frozen types/traits/constants (Manifest, Entry, ChunkRef, RefRecord) all other clw crates depend on | reuse-verbatim | yes | The manifest/ref vocabulary the runner needs is already frozen; hugit consumes it, does not fork it. |
| 5 | `clw-chunk` FastCDC chunker | `../corelink-workspaces/docs/ARCHITECTURE.md` (Snapshot dataflow); `../corelink-workspaces/crates/clw-chunk` | Content-defined chunking 256 KiB/1 MiB/4 MiB with CAS dedup-on-exists | reuse-verbatim | yes | Chunking/dedup is pure CAS plumbing the runner inherits unchanged via clw. |
| 6 | `clw-manifest` sparse manifest | `../corelink-workspaces/docs/ARCHITECTURE.md` (crate graph + Snapshot); `../corelink-workspaces/crates/clw-manifest` | Sorted-by-path Entry{File/Symlink/Dir}, canonical-bytes, BLAKE3 manifest digest | adapt | yes | The manifest is the raw material for hugit's `FenceManifest` (claim-filtered sparse materialization, WP-00 §6); C5a adds the path-set/deny-default filter on top. |
| 7 | `clw-cache` local content cache | `../corelink-workspaces/docs/ARCHITECTURE.md` (crate graph) | Local digest-keyed cache that warms across hydrations | reuse-verbatim | yes | The warm-boot cache C3 needs already exists; no change. |
| 8 | CoreLink CAS (`/v1/cas`) | `../corelink-server/ARCHITECTURE.md` (REAPI layer); `../corelink-server/crates/corelink-cas/src/` (chunker, dedup, manifest, r2_multipart) | Content-addressed blob store on R2 with dedup, multipart, eviction, integrity | reuse-verbatim | yes | Consumed as a customer over the live API (per warp-10-days provisioning); no server change — the fabric stores artifacts here. |
| 9 | CoreLink Action Cache (`corelink-ac`, `/v1/ac`) | `../corelink-server/ARCHITECTURE.md` §5.3 (Action-cache association); `../corelink-server/crates/corelink-ac/` | UpdateActionResult digest→outputs binding; content-uniform 404; AC-meta freshness | reuse-verbatim | yes | Checks-as-code (B2) and `clw run` memo both ride this AC; the runner reports results into it. |
| 10 | REAPI v2 wire surface (`corelink-reapi`) | `../corelink-server/crates/corelink-reapi/src/lib.rs:49-53,109-131` | gRPC `ContentAddressableStorage`, `ByteStream` (read+write), `Capabilities` services + capabilities/error-map logic | reuse-verbatim | yes | The cache-side REAPI services are built and consumable; hugit talks the same wire to store/fetch action data. |
| 11 | REAPI **Execution** service (`Execute`/`WaitExecution`/worker) | `../corelink-server/crates/corelink-reapi/src/lib.rs` (services enumerated = CAS/ByteStream/Capabilities only; **no ExecutionService**) | The actual remote-execution worker that runs an Action on a box | build-new | yes (container-per-job is the new build) | Verified absent: REAPI ships the cache side, not the execution side — this is precisely the L3 gap C2 fills (ephemeral runner v0 on the Hetzner box). |
| 12 | `RunnerLease` lifecycle (held/expired/crashed/released, path_set, expiry, net_policy, tmp_root) | `docs/plan/wp-contracts/WP-00.md` §5 (frozen shape); no CoreLink-side implementation found while inspecting `corelink-reapi`/`corelink-pat` | The lease object the runner acquires and the box enforces | build-new | yes | The type is frozen on the hugit side (WP-00); campaign #1 has no lease lifecycle to reuse — C2a/C2b build it on the new runner. |
| 13 | Workspace fence / sparse-by-path-set isolation | `../corelink-workspaces/docs/ARCHITECTURE.md` (Security notes: tenant-prefix isolation, integrity-verify); `../corelink-workspaces/crates/clw-run/src/lib.rs:184-189` (bare child spawn) | Tenant-prefix privacy at the CAS layer; but no per-job path-set fence or net isolation on the executor | build-new | yes | clw isolates *tenants at storage*, not *jobs at the box* — the claim-fenced sparse-hydrate fence (C5a: outside-path-set→ENOENT) does not exist yet; build it. |
| 14 | Secrets broker / write-only secret model (runner never holds credentials) | `../corelink-server/crates/corelink-byok/src/` (byok_core/aws/azure/gcp/vault/revocation); `docs/whitepaper/hugit-v1.md` §9 (broker generalization) | BYOK = customer-managed *encryption keys* (KMS adapters), the model to generalize — but no runner-side credential broker | build-new | yes | BYOK proves the write-only pattern exists for KMS, but the runner secrets broker (C5b: zero secret material in job, broker-mediated, fail-closed) is unbuilt — generalize, don't reuse the BYOK crate verbatim. |
| 15 | PAT auth + scopes (`corelink-pat`) | `../corelink-server/crates/corelink-pat/src/scopes.rs:56-58,167-168` | Argon-hashed PATs, u64 scope bitset incl. `SCOPE_EXECUTE_ACTION` ("execute:action") + `SCOPE_REPORT_RESULT` ("report:result"), 51 reserved bits | reuse-verbatim | yes | The runner-execution scopes are already minted in the frozen PAT catalog; the lease's principal chain authenticates with these unchanged. |
| 16 | Firecracker / microVM isolation | `../corelink-server/crates/corelink-container/Cargo.toml`, `src/storage.rs`, `src/storage/d1_http.rs` (only incidental "firecracker" mentions; no microVM execution layer) | — | build-new | **no — Firecracker-only, deferred** | No Firecracker execution exists in campaign #1; per WP-C1 the upgrade path is *documented, not built* — container-per-job is the now-build, microVM is deferred. |
| 17 | `corelink-chaos-scheduler` | `../corelink-server/crates/corelink-chaos-scheduler/Cargo.toml` | CoreLink's own SRE chaos-experiment harness (latency/failure/resource/partition) | build-new | n/a | Not runner work — it is CoreLink's internal reliability tooling; no reuse for the hugit runner fabric (recorded here so it is not mistaken for execution substrate). |

## Verdict summary

| Verdict | Count | Items |
|---|---|---|
| reuse-verbatim | 9 | 1, 2, 4, 5, 7, 8, 9, 10, 15 |
| adapt | 2 | 3, 6 |
| build-new | 6 | 11, 12, 13, 14, 16, 17 |
| **total** | **17** | |

## The shape of campaign-1 reuse (one paragraph)

CoreLink shipped the **entire cache/storage half** of a remote-execution
fabric — CAS, Action Cache, ByteStream, Capabilities (REAPI v2), the FastCDC
chunker/dedup, content-addressed manifests, the warm local cache, the `clw`
snapshot/hydrate/memoized-run client, PAT auth with `execute:action` /
`report:result` scopes already minted, and tenant-prefix isolation. What it did
**not** build is the **execution half**: there is no REAPI Execution service, no
`RunnerLease` lifecycle, no per-job box-level fence or net isolation, and no
runner secrets broker. `clw run` does memoized execution but spawns a bare child
process with no isolation. So the C-squad's job is precise: **reuse the
cache/storage substrate verbatim, adapt the memo executor and manifest, and
build the thin execution+isolation+lease+broker layer new** — container-per-job
on the Hetzner box now, Firecracker documented for later.

# P2 go-live runbook — flipping the live-infra seams

> **Purpose.** Every P2-gated seam is built, hermetically proven, and wired
> **fail-closed behind an env gate that runs-not-skips when the env is set**.
> This runbook is the single, exact, tested sequence to flip them live the moment
> the provisioning lands — no scramble, no rediscovery. Each test goes RED if its
> seam is wired wrong, and SKIPs cleanly (printed reason) when its env is absent,
> so nothing can rot to green before go-live.
>
> Run from the runner box `hugit-runner-01` (91.99.11.196) where the PAT lives.
> Toolchain: 1.96.0 (`rust-toolchain.toml`). All commands are `--locked`.

## Provisioning prerequisites (owner-gated, not code)

| Dep | What | State |
|---|---|---|
| **P1** | GitHub App (id, private key, install token) | ✅ done |
| **P2** | CoreLink pilot tenant: AC URL + slug + PAT (`cas:rw`), caps | ⏳ pending — see `2026-06-08-corelink-p2-tenant-request.md` + `…-techlead-questions.md` |
| **P3** | Hetzner runner box `hugit-runner-01` (91.99.11.196) | ✅ done |

**Order:** provision P2 **after** the CoreLink prod launch + audit + deploy (do not
smoke-test against the stale pre-audit container — agreed in the Q&A §8). Groups A→D
below are independent; run each as its inputs become available.

---

## Group A — CoreLink AC tenant (the wedge)

Set once (PAT preferably as the file, mode 600):

```sh
export HUGIT_CORELINK_AC_URL="https://corelink-api.humangr.com"   # the FLAT host
export HUGIT_CORELINK_TENANT="hugit"
# PAT: ~/.hugit/secrets/corelink/pat (preferred) OR export HUGIT_CORELINK_PAT=…
```

| Seam | WP | Command | Pass = |
|---|---|---|---|
| AC HTTP smoke (3 probes: 404 miss → round-trip hit → 403 cross-tenant) | B2a | `cargo test -p hugit-checks --test corelink_ac_smoke -- --nocapture` (`corelink_ac_live_smoke`) | three probes pass |
| Intra-fabric live measure (CoreLink latency unaffected under hugit load) | X6 | `HUGIT_CORELINK_PROBE_URL=$HUGIT_CORELINK_AC_URL cargo test -p hugit-invariants --test acceptance_x6` (`item_1b_live_measurement…`) | latency delta within bound |
| Focus-gate live measure (other tenants unaffected; consumption cap honored) | X10 | `HUGIT_X10_LIVE=1 HUGIT_CORELINK_PROD_URL=$HUGIT_CORELINK_AC_URL HUGIT_CORELINK_TENANT_CAP=<cap> cargo test -p hugit-invariants --test acceptance_x10` (`item_1b/4b`) | no cross-tenant impact |

## Group B — runner box (`HUGIT_RUNNER_HOST`)

```sh
export HUGIT_RUNNER_HOST="91.99.11.196"   # hugit-runner-01
```

| Seam | WP | Command |
|---|---|---|
| Runner-side byte-identity (content-digest) | B2b | `cargo test -p hugit-checks --test acceptance_b2b` (`live_runner_box_seam_documented_and_gated`) |
| Cache-warm boot (warm vs cold timing) | C3 | `cargo test -p hugit-runner --test acceptance_c3` |
| Lease lifecycle / isolation / load / ws lifecycle / Actions shim | C2a C2b C9 E4 | `cargo test -p hugit-runner --test acceptance_c2a --test acceptance_c2b --test acceptance_c9 --test acceptance_e4` |
| Secrets broker (live escape red-team) | C5b | `cargo test -p hugit-fence --test acceptance_c5b` |
| Degradation: mid-op broker fault on the box | X11 | `cargo test -p hugit-invariants --test acceptance_x11` (`live_seam_mid_op_fault_on_box`) |

## Group C — live GitHub (App install token + a test repo)

```sh
export HUGIT_GH_TEST_REPO="<owner>/<throwaway-repo>"
export HUGIT_GH_INSTALL_TOKEN="<short-lived install token>"   # for history import
export HUGIT_QUEUE_AUTOTRIGGER=1                              # B5 auto-bisect on red
```

| Seam | WP | Command |
|---|---|---|
| Branch-protection / merge API / force-push recompute + queue auto-trigger | B4b B5 | `cargo test -p hugit-queue --test acceptance_wp-b4b` (`item_6_live_app_jwt_lists_installations`) |
| Bidirectional sync: live GitHub-side change detect (webhook/poll) | bidir (E6-superseded) | `cargo test -p hugit-mirror --test acceptance_bidir` (`p2_live_github_detect_seam_gated`) |
| History import (byte-identity, LFS, resumable) | E2a | `cargo test -p hugit-mirror --test acceptance_e2a` |

## Group D — soak (wall-clock, after a live install)

```sh
export HUGIT_DOGFOOD_LIVE=1
```

| Seam | WP | Command |
|---|---|---|
| Dogfood 48h soak invariant (real PR wave, memoization honest hit-rate) | B8 | `cargo test -p hugit-dogfood --test acceptance_b8` (`item_3_live_48h_soak_p2_seam_run_not_skip`) |

---

## Verify-all (once all groups are wired)

With every env above exported, a full gated run executes (not skips) the live seams:

```sh
cargo test --workspace --locked -- --nocapture 2>&1 | tee /tmp/p2-golive.log
grep -cE 'SKIP|LIVE-SKIP' /tmp/p2-golive.log   # expect 0 once fully provisioned
grep -E 'test result:' /tmp/p2-golive.log      # expect all ok, 0 failed
```

**Done-when:** zero `SKIP`/`LIVE-SKIP` lines remain and every suite is `ok`. At that
point the disclosed P2 seams are live and task #7 closes. Until then each unset gate
skips with a printed reason — never a false green.

## Notes

- **X8 self-release transparency log** is internal (ed25519 + boot self-verify); its
  public-log anchoring is operational, not a `cargo test` env seam — verify at release
  time, not here.
- Env names are the **frozen constants** in code (`HUGIT_CORELINK_AC_URL`,
  `HUGIT_RUNNER_HOST`, `HUGIT_GH_TEST_REPO`, `HUGIT_DOGFOOD_LIVE`, `HUGIT_X10_LIVE`, …);
  this runbook and the code cannot drift.
- GitHub Actions cloud CI is quota-paused; this runbook is local cold-verify. Re-enable
  the cloud `gates` job when quota is restored (unrelated to P2).

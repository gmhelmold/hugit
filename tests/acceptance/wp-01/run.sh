#!/usr/bin/env bash
# WP-01 acceptance suite — workspace scaffold (12 crates, CI gates green on
# empty, DCO + changelog config). Contract: docs/plan/wp-contracts/WP-01.md.
# RED on a non-workspace tree; GREEN only when the scaffold lands.
#
# Lead adjudication on record (wave-day0): hugit-contracts joins the workspace
# via the members glob when WP-00 creates it; WP-01 wires NO per-crate deps on
# it (cannot compile before WP-00). Downstream WPs add their own dep.

SUITE_ID="wp-01"
export SUITE_ID
source "$(dirname "$0")/../lib.sh"

CRATES=(hugit-app hugit-queue hugit-checks hugit-diag hugit-runner hugit-fence
        hugit-refstore hugit-proto hugit-ledger hugit-policy hugit-mirror hugit-cli)

# ── ① virtual workspace root, glob members, resolver 2 ─────────────────────
check "workspace Cargo.toml exists" test -f Cargo.toml
check "Cargo.toml declares [workspace]" grep -q '^\[workspace\]' Cargo.toml
check "members use the crates/* glob (shared-file elimination)" \
  grep -q 'members *= *\[ *"crates/\*" *\]' Cargo.toml
check "resolver = \"2\"" grep -q 'resolver *= *"2"' Cargo.toml
check "virtual workspace (no [package] at root)" \
  bash -c '! grep -q "^\[package\]" Cargo.toml'

# ── ② exactly the 12 named crates, each an empty lib stub ───────────────────
for c in "${CRATES[@]}"; do
  check "crate $c present (Cargo.toml + src/lib.rs)" \
    bash -c "test -f crates/$c/Cargo.toml && test -f crates/$c/src/lib.rs"
done
check "no crates beyond the named set (+hugit-contracts allowed)" bash -c '
  for d in crates/*/; do
    n=$(basename "$d")
    case " hugit-app hugit-queue hugit-checks hugit-diag hugit-runner hugit-fence hugit-refstore hugit-proto hugit-ledger hugit-policy hugit-mirror hugit-cli hugit-contracts " in
      *" $n "*) ;;
      *) exit 1 ;;
    esac
  done'

# ── ③ toolchain pin + runner-protection cargo config ────────────────────────
check "rust-toolchain.toml pins the toolchain" \
  bash -c 'test -f rust-toolchain.toml && grep -q "channel" rust-toolchain.toml'
check ".cargo/config.toml caps build jobs at 4" \
  bash -c 'test -f .cargo/config.toml && grep -Eq "jobs *= *4" .cargo/config.toml'

# ── ④ the four gates, green on the empty scaffold ───────────────────────────
check "gate: cargo fmt --check" cargo fmt --all --check
check "gate: cargo clippy -D warnings" \
  cargo clippy --workspace --all-targets -- -D warnings
check "gate: cargo test" cargo test --workspace
check "gate: cargo audit" cargo audit

# ── ⑤ CI + DCO + changelog discipline ───────────────────────────────────────
check "ci.yml runs all four gates" bash -c '
  test -f .github/workflows/ci.yml &&
  grep -q "fmt" .github/workflows/ci.yml &&
  grep -q "clippy" .github/workflows/ci.yml &&
  grep -q "test" .github/workflows/ci.yml &&
  grep -q "audit" .github/workflows/ci.yml'
check "dco.yml enforces Signed-off-by" bash -c '
  test -f .github/workflows/dco.yml &&
  grep -qi "signed-off-by" .github/workflows/dco.yml'
check "CHANGELOG.md with [Unreleased] section" bash -c '
  test -f CHANGELOG.md && grep -q "\[Unreleased\]" CHANGELOG.md'
check "changelog gate script present + executable" bash -c '
  test -x .github/scripts/changelog-gate.sh'

finish

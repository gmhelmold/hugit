# githugr admin-area — transport bundle

> **Authored in hugit (this dir), to be TRANSPORTED into `../githugr` by the
> githugr session/owner when its tree is free.** Built here because githugr is an
> actively-edited sibling tree — writing there concurrently got my work wiped by a
> `git checkout/reset` (the banked incident). The engine half is already SHIPPED on
> hugit `main` (PR #123: `GET /v1/repos/{repo}/{audit,erasure,admin/overview}`).
>
> This is the **UI half**: the operator admin area at `/admin`, account-level,
> consuming the three engine reads. Pure maud screen + provider plumbing — it
> CANNOT compile in hugit (githugr stack), so it is verified by githugr's own
> `cargo test` + CI at transport time. Authored against the githugr patterns
> (axum 0.8 + maud 0.27, kit.css tokens, VM→screen→route→fixture→live→hybrid).

## What it adds

An account-level `/admin` operator console (page chrome = `layout::page_account`,
hybrid → live). One screen composing the engine's three admin reads:
- **Overview** — KPI snapshot (queue depth, active campaigns, attention, total PRs,
  active policy rules, erasure decisions, log depth, last-activity age).
- **Audit timeline** — recent events newest-first (seq · kind · principal ·
  scrubbed summary · age · integrity hash_short). Raw payload never shown.
- **Erasure governance** — approved + denied decisions; execution always `pending`
  (X12 = P2 seam).

## Transport checklist (mechanical — 8 edits)

### 1. NEW FILE → `crates/githugr-screens/src/screens/admin.rs`
Copy `admin.rs` from this dir verbatim.

### 2. `crates/githugr-screens/src/screens/mod.rs`
After `pub mod account;` add:
```rust
pub mod admin;
```

### 3. `crates/githugr-vm/src/provider.rs`
(a) In `trait Provider`, after the `attention` method, add:
```rust
    /// The operator admin area (`/admin`) — the forge control-plane snapshot:
    /// overview + recent audit timeline + erasure governance.
    async fn admin(&self) -> Result<AdminVm, EngineUnavailable>;
```
(b) Append the VM structs (mirror the engine wire shapes field-for-field):
```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AdminOverviewVm {
    pub queue_depth: usize,
    pub active_campaigns: usize,
    pub attention_count: usize,
    pub total_prs: usize,
    pub policy_rules_active: usize,
    pub erasure_decisions: usize,
    pub log_depth: u64,
    pub last_activity_age: String,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuditEntryVm {
    pub seq: u64,
    pub kind: String,
    pub principal: String,
    pub summary: String,
    pub age: String,
    pub recorded_at: u64,
    pub hash_short: String,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuditVm {
    pub entries: Vec<AuditEntryVm>,
    pub returned: usize,
    pub next_since: Option<u64>,
    pub head_seq: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ErasureRowVm {
    pub erasure_id: String,
    pub state: String,
    pub execution: String,
    pub decided_by: String,
    pub age: String,
    pub seq: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ErasureHistoryVm {
    pub entries: Vec<ErasureRowVm>,
    pub approved_count: usize,
    pub denied_count: usize,
    pub note: String,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AdminVm {
    pub overview: AdminOverviewVm,
    /// Recent audit rows, NEWEST FIRST (screen reverses the engine's ascending tail).
    pub recent_audit: Vec<AuditEntryVm>,
    pub erasure: ErasureHistoryVm,
}
```

### 4. `crates/githugr/src/routes.rs`
(a) After `.route("/attention", get(attention))` add:
```rust
        .route("/admin", get(admin))
```
(b) Before `async fn attention(` add the handler:
```rust
async fn admin(
    State(p): State<AnyProvider>,
    Query(flash): Query<FlashParams>,
) -> Result<Response, AppError> {
    let vm = p.admin().await.map_err(AppError::engine)?;
    Ok(ok(layout::page_account(
        "Admin",
        vm.overview.attention_count,
        p.is_fixture().await,
        screens::admin::SCREEN_CSS,
        screens::admin::render(&vm),
        flash.into_flash(),
        &[("⌘K", "comandos")],
        "operador · githugr · versionado por hugit",
    )))
}
```

### 5. `crates/githugr-fixture/src/fixture.rs`
(a) In `impl Provider for FixtureProvider`, after `attention()`:
```rust
    async fn admin(&self) -> Result<AdminVm, EngineUnavailable> {
        Ok(w2_admin_vm())
    }
```
(b) Add the seeded builder (near `w2_dashboard_vm`):
```rust
fn w2_admin_vm() -> AdminVm {
    let audit = |seq: u64, kind: &str, principal: &str, summary: &str, age: &str, hash: &str| {
        AuditEntryVm { seq, kind: kind.to_string(), principal: principal.to_string(),
            summary: summary.to_string(), age: age.to_string(), recorded_at: 0, hash_short: hash.to_string() }
    };
    AdminVm {
        overview: AdminOverviewVm { queue_depth: 3, active_campaigns: 2, attention_count: 2,
            total_prs: 14, policy_rules_active: 3, erasure_decisions: 2, log_depth: 217,
            last_activity_age: "há 12 min".to_string() },
        recent_audit: vec![
            audit(216, "pr.queued", "orchestrator:fleet", "PR #134 na fila", "há 12 min", "a1b2c3d4e5f6"),
            audit(215, "verdict.recorded", "orchestrator:panel", "veredito approve", "há 18 min", "b2c3d4e5f6a7"),
            audit(214, "policy.set", "human:gustavo", "regra 'changelog'", "há 1 h", "c3d4e5f6a7b8"),
            audit(213, "check.recorded", "orchestrator:fleet", "check 'clippy'", "há 1 h", "d4e5f6a7b8c9"),
            audit(212, "pr.landed", "orchestrator:fleet", "PR #131", "há 2 h", "e5f6a7b8c9d0"),
        ],
        erasure: ErasureHistoryVm {
            entries: vec![
                ErasureRowVm { erasure_id: "er-7".to_string(), state: "approved".to_string(),
                    execution: "pending".to_string(), decided_by: "human:gustavo".to_string(), age: "há 3 h".to_string(), seq: 208 },
                ErasureRowVm { erasure_id: "er-4".to_string(), state: "denied".to_string(),
                    execution: "pending".to_string(), decided_by: "human:gustavo".to_string(), age: "há 1 d".to_string(), seq: 190 },
            ],
            approved_count: 1, denied_count: 1,
            note: "execução de uma erasure aprovada é o seam P2 (CAS-scrub); aqui só a decisão é registrada".to_string(),
        },
    }
}
```

### 6. `crates/githugr-live/src/provider.rs`
In `impl Provider for LiveProvider`, after `attention()`:
```rust
    async fn admin(&self) -> Result<AdminVm, EngineUnavailable> {
        let repo = self.default_repo().await;
        let unavail = |what: &str| EngineUnavailable { detail: format!("admin: {what} absent for repo {repo}") };
        let overview: AdminOverviewVm = self
            .get_opt_q(&format!("repos/{repo}/admin/overview"), &[]).await?
            .ok_or_else(|| unavail("overview"))?;
        let since = overview.log_depth.saturating_sub(20).to_string();
        let audit: AuditVm = self
            .get_opt_q(&format!("repos/{repo}/audit"), &[("since", since.as_str()), ("limit", "20")]).await?
            .ok_or_else(|| unavail("audit"))?;
        let erasure: ErasureHistoryVm = self
            .get_opt_q(&format!("repos/{repo}/erasure"), &[]).await?
            .ok_or_else(|| unavail("erasure"))?;
        let mut recent_audit = audit.entries;
        recent_audit.reverse(); // engine ascending → screen newest-first
        Ok(AdminVm { overview, recent_audit, erasure })
    }
```
(`get_opt_q` already exists; it attaches the query via reqwest `.query()`.)

### 7. `crates/githugr/src/any.rs`
In `impl Provider for AnyProvider`, after the `attention` match arm:
```rust
    async fn admin(&self) -> Result<AdminVm, EngineUnavailable> {
        match self {
            Self::Fixture(p) => p.admin().await,
            Self::Live(p) => p.admin().await,
            Self::Hybrid(p) => p.admin().await,
        }
    }
```

### 8. `crates/githugr/src/hybrid.rs`
(a) In `impl Provider for HybridProvider`, after `attention()` (account-level → live):
```rust
    async fn admin(&self) -> Result<AdminVm, EngineUnavailable> {
        self.live.admin().await
    }
```
(b) In `LIVE_SET`, after `"attention",` add `"admin",`.
(c) Add `AdminVm` (and the nested VMs if not glob-imported) to the `use githugr_vm::{…}` list.

## Verify at transport
```
cargo fmt -p githugr-vm -p githugr-screens -p githugr -p githugr-fixture -p githugr-live
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
Add a snapshot test for the screen (mirror an existing `screen_*` test) if the
repo's convention requires one per screen.

## Engine endpoints this consumes (already live on hugit `main`)
- `GET /v1/repos/{repo}/admin/overview` → `AdminOverviewVm`
- `GET /v1/repos/{repo}/audit?since=&limit=&kind=&principal=` → `AuditVm`
- `GET /v1/repos/{repo}/erasure` → `ErasureHistoryVm`

//! screen_checks — acceptance tests for the W5 Checks screen render.
//!
//! All VMs are hand-built here; fixture.rs is NOT used.
//! Tests assert on `render(&vm).into_string()`.

use hugit_web::provider::{BisectVm, CheckRowVm, ChecksKpisVm, ChecksVm};
use hugit_web::screens::checks::render;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn kpis(
    hit_rate_pct: f64,
    shape: &str,
    hits: usize,
    executed: usize,
    saved_ms: u64,
) -> ChecksKpisVm {
    ChecksKpisVm {
        hit_rate_pct,
        shape: shape.to_string(),
        hits,
        executed,
        saved_ms,
    }
}

fn vm_minimal(hit_rate_pct: f64, shape: &str) -> ChecksVm {
    ChecksVm {
        repo: "testrepo".to_string(),
        kpis: kpis(hit_rate_pct, shape, 0, 0, 0),
        checks: vec![],
        bisect: None,
        memo_note: "byte-idêntico — memo note".to_string(),
    }
}

fn cache_hit_row() -> CheckRowVm {
    CheckRowVm {
        name: "fmt".to_string(),
        ok: true,
        duration_ms: 0,
        cache_hit: true,
        log: "sem execução — resultado recuperado do cache CAS\nhash de entrada: sha256:abc123"
            .to_string(),
        memo_key: "sha256:abc123deadbeef0011223344556677889900aabbccddeeff".to_string(),
    }
}

fn executed_row() -> CheckRowVm {
    CheckRowVm {
        name: "test --workspace".to_string(),
        ok: true,
        duration_ms: 4200,
        cache_hit: false,
        log: "running 18 tests\ntest result: ok. 18 passed; 0 failed".to_string(),
        memo_key: "sha256:feeddeadbeef1234567890abcdef1234567890abcdef1234".to_string(),
    }
}

fn failing_row() -> CheckRowVm {
    CheckRowVm {
        name: "clippy".to_string(),
        ok: false,
        duration_ms: 800,
        cache_hit: false,
        log: "error[E0001]: some lint error".to_string(),
        memo_key: "sha256:badcafe0000111222333444555666777888999aaabbbccc0".to_string(),
    }
}

fn bisect_vm() -> BisectVm {
    BisectVm {
        culprit: "a2c491".to_string(),
        probes: 3,
        max_probes: 8,
        steps: vec![
            "testar em a31f9c — commit mais recente".to_string(),
            "testar em b3e810 — meio do range".to_string(),
            "testar em a2c491 — commit anterior".to_string(),
        ],
    }
}

// ---------------------------------------------------------------------------
// Test 1 — KPI hit_rate_pct binds with one decimal; shape string verbatim
// ---------------------------------------------------------------------------

#[test]
fn kpis_bind_hit_rate_and_shape() {
    let vm = ChecksVm {
        repo: "testrepo".to_string(),
        kpis: kpis(37.5, "PARTIAL", 3, 5, 14_000),
        checks: vec![],
        bisect: None,
        memo_note: "memo".to_string(),
    };
    let html = render(&vm).into_string();

    // hit-rate rendered with one decimal
    assert!(
        html.contains("37.5"),
        "expected hit-rate '37.5' in output, got: {html}"
    );

    // shape string verbatim AS-IS
    assert!(
        html.contains("PARTIAL"),
        "expected shape 'PARTIAL' verbatim in output"
    );
}

// ---------------------------------------------------------------------------
// Test 2 — cache-hit row and executed row render DIFFERENT affordances
// ---------------------------------------------------------------------------

#[test]
fn cache_hit_and_executed_rows_differ() {
    let vm = ChecksVm {
        repo: "testrepo".to_string(),
        kpis: kpis(50.0, "PARTIAL", 1, 1, 0),
        checks: vec![cache_hit_row(), executed_row()],
        bisect: None,
        memo_note: "memo".to_string(),
    };
    let html = render(&vm).into_string();

    // cache-hit row uses "cache-hit" label
    assert!(
        html.contains("cache-hit"),
        "expected 'cache-hit' affordance label in output"
    );

    // executed row uses "executou" label
    assert!(
        html.contains("executou"),
        "expected 'executou' affordance label in output"
    );

    // Both must be present — they differ
    assert!(
        html.contains("cache-hit") && html.contains("executou"),
        "both cache-hit and executou affordances must appear simultaneously"
    );

    // Status classes differ: "st hit" vs "st ran"
    assert!(
        html.contains("st hit"),
        "cache-hit row must use 'st hit' class"
    );
    assert!(
        html.contains("st ran"),
        "executed row must use 'st ran' class"
    );
}

// ---------------------------------------------------------------------------
// Test 3 — log content appears in log pane; memo_key truncated with title=full
// ---------------------------------------------------------------------------

#[test]
fn log_content_and_memo_key_binding() {
    let row = cache_hit_row();
    let full_key = row.memo_key.clone();
    let vm = ChecksVm {
        repo: "testrepo".to_string(),
        kpis: kpis(100.0, "FULL", 1, 0, 1000),
        checks: vec![row],
        bisect: None,
        memo_note: "memo".to_string(),
    };
    let html = render(&vm).into_string();

    // Log text must appear
    assert!(
        html.contains("resultado recuperado do cache CAS"),
        "first log line must appear in the log pane"
    );
    assert!(
        html.contains("hash de entrada"),
        "second log line must appear"
    );

    // Full memo key in title attribute
    assert!(
        html.contains(&full_key),
        "full memo key must appear in title attribute: {full_key}"
    );

    // Truncated key (first 16 chars + ellipsis) as visible text
    let truncated = format!("{}…", &full_key[..16]);
    assert!(
        html.contains(&truncated),
        "truncated memo key must appear as visible text: {truncated}"
    );
}

// ---------------------------------------------------------------------------
// Test 4a — bisect Some renders culprit + "log₂" copy
// ---------------------------------------------------------------------------

#[test]
fn bisect_some_renders_culprit_and_log2_copy() {
    let vm = ChecksVm {
        repo: "testrepo".to_string(),
        kpis: kpis(0.0, "NONE", 0, 3, 0),
        checks: vec![],
        bisect: Some(bisect_vm()),
        memo_note: "memo".to_string(),
    };
    let html = render(&vm).into_string();

    // Culprit commit appears
    assert!(
        html.contains("a2c491"),
        "culprit commit must appear in bisect block"
    );

    // The log₂ copy from the mockup (using Unicode subscript 2)
    assert!(
        html.contains("log\u{2082}") || html.contains("log₂"),
        "≤ log₂ copy must appear in bisect block"
    );
}

// ---------------------------------------------------------------------------
// Test 4b — bisect None renders NO bisect block
// ---------------------------------------------------------------------------

#[test]
fn bisect_none_renders_no_bisect_block() {
    let vm = vm_minimal(100.0, "FULL");
    let html = render(&vm).into_string();

    assert!(
        !html.contains("Auto-bisect"),
        "bisect block must NOT appear when vm.bisect is None"
    );
    assert!(
        !html.contains("culpado"),
        "culprit content must NOT appear when vm.bisect is None"
    );
}

// ---------------------------------------------------------------------------
// Test 5 — memo_note renders
// ---------------------------------------------------------------------------

#[test]
fn memo_note_renders() {
    let vm = ChecksVm {
        repo: "testrepo".to_string(),
        kpis: kpis(91.0, "FULL", 11, 3, 840_000),
        checks: vec![],
        bisect: None,
        memo_note: "byte-idêntico — o hugit define \"mesmo input\" como o hash CAS do código-fonte + toolchain + flags."
            .to_string(),
    };
    let html = render(&vm).into_string();

    assert!(
        html.contains("byte-idêntico"),
        "memo_note must render in the memonote block"
    );
    assert!(html.contains("hash CAS"), "full memo_note text must appear");
}

// ---------------------------------------------------------------------------
// Test 6 — failing row renders failure affordance
// ---------------------------------------------------------------------------

#[test]
fn failing_row_renders_failure_affordance() {
    let vm = ChecksVm {
        repo: "testrepo".to_string(),
        kpis: kpis(0.0, "NONE", 0, 1, 0),
        checks: vec![failing_row()],
        bisect: None,
        memo_note: "memo".to_string(),
    };
    let html = render(&vm).into_string();

    // Failure label
    assert!(
        html.contains("falhou"),
        "failing row must render 'falhou' label"
    );

    // Failure class "st fail"
    assert!(
        html.contains("st fail"),
        "failing row must use 'st fail' class"
    );

    // The status div for a failing row must use "fail" class, not "hit"
    // (Note: "cache-hit" may appear in the section header counter; we check the status class)
    assert!(
        !html.contains("st hit"),
        "failing row must NOT use 'st hit' class"
    );
}

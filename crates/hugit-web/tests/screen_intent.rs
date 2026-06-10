//! WP-W4 screen test — the Intent detail render, asserted against hand-built
//! view-models (NOT the fixture world). Each test pins one faithfulness clause
//! from the spec; the honesty clause (a `None` accordion renders the explicit
//! "não capturado" state, never a blank body) is load-bearing and gets its own
//! assertions. The app_smoke oracle covers the live fixture route separately.

use hugit_web::provider::{
    AuthorshipVm, DiffLineKind, DiffLineVm, DiffVm, FileRowVm, HunkVm, IntentDetailVm, MetricsVm,
    SnapshotVm, VerdictVm,
};
use hugit_web::screens::intent::render;

/// A rich intent with every accordion captured, a real diff, a principal chain
/// of three, and a marked adversarial verdict.
fn captured_vm() -> IntentDetailVm {
    IntentDetailVm {
        repo: "corelink-server".to_string(),
        id: "a31".to_string(),
        title: "fix: sessão expira cedo no refresh do token".to_string(),
        status: "pousou na main".to_string(),
        pr_number: Some(128),
        summary: "Re-derivou iat de now() em token.rs:42 para corrigir a janela.".to_string(),
        charter: "Refresh reusava o iat do token antigo → janela curta.".to_string(),
        acceptance: vec![
            "a sessão dura o TTL completo após refresh".to_string(),
            "sem reemissão sem credencial; não tocar billing/".to_string(),
        ],
        task_transcript: Some("brief recebido · passo 1 leitura · passo 2 edição".to_string()),
        full_transcript: Some("turn 1 system+charter · turn 3 Read token.rs".to_string()),
        journal: Some("conferi a re-emissão pra deslogado — ok pra pousar.".to_string()),
        context_json: "{\n  \"schema_version\": \"1.0.0\",\n  \"intent_id\": \"a31\"\n}"
            .to_string(),
        diff: DiffVm {
            files: vec![FileRowVm {
                path: "crates/auth/src/token.rs".to_string(),
                added: 12,
                removed: 4,
            }],
            hunks: vec![HunkVm {
                file: "crates/auth/src/token.rs".to_string(),
                header: "@@ -40,5 +40,5 @@".to_string(),
                lines: vec![
                    DiffLineVm {
                        kind: DiffLineKind::Context,
                        text: "  pub fn refresh(old: &Token) -> Token {".to_string(),
                    },
                    DiffLineVm {
                        kind: DiffLineKind::Del,
                        text: "-     let iat = old.iat;".to_string(),
                    },
                    DiffLineVm {
                        kind: DiffLineKind::Add,
                        text: "+     let iat = now();".to_string(),
                    },
                ],
            }],
        },
        authorship: AuthorshipVm {
            model: "opus-4.8".to_string(),
            principal_chain: vec![
                "owner".to_string(),
                "orq-014".to_string(),
                "r-9f2a".to_string(),
            ],
            operator: "gustavo@humangr.com".to_string(),
        },
        metrics: MetricsVm {
            tokens: 38200,
            wall_ms: 877000,
            tool_calls: 14,
            cost_usd: 0.04,
        },
        snapshot: SnapshotVm {
            tree: "7e1abcdef0123456789abcdef0123456789abcdef0123456789abcdef01234567".to_string(),
            toolchain: "rustc 1.96.0".to_string(),
            workspace: "ws-9f2a".to_string(),
        },
        verdicts: vec![
            VerdictVm {
                verdict: "APPROVE".to_string(),
                reviewer: "correctness".to_string(),
                summary: "iat re-derivado; TTL pleno".to_string(),
                adversarial: true,
            },
            VerdictVm {
                verdict: "APPROVE".to_string(),
                reviewer: "security".to_string(),
                summary: "sem caminho sem credencial".to_string(),
                adversarial: true,
            },
        ],
    }
}

/// The same intent stripped of every captured trajectory body — exercises the
/// honest "não capturado" path on all three accordions.
fn uncaptured_vm() -> IntentDetailVm {
    IntentDetailVm {
        task_transcript: None,
        full_transcript: None,
        journal: None,
        ..captured_vm()
    }
}

const NOT_CAPTURED: &str = "não capturado neste intent (nível de captura / WP-F2 pendente)";

// 1 — header binds id/title/status; breadcrumb links to landing.
#[test]
fn header_and_breadcrumb_bind() {
    let html = render(&captured_vm()).into_string();
    assert!(html.contains("a31"), "intent id");
    assert!(
        html.contains("fix: sessão expira cedo no refresh do token"),
        "title"
    );
    assert!(html.contains("pousou na main"), "status badge");
    // Landing + PR crumbs both link to the landing route; PR shows #128.
    assert!(
        html.contains("href=\"/r/corelink-server/landing\""),
        "breadcrumb links to landing"
    );
    assert!(html.contains("PR #128"), "PR crumb");
}

// 2 — charter + every acceptance item render.
#[test]
fn charter_and_every_acceptance_render() {
    let vm = captured_vm();
    let html = render(&vm).into_string();
    assert!(
        html.contains("Refresh reusava o iat do token antigo"),
        "charter prose"
    );
    for item in &vm.acceptance {
        assert!(html.contains(item.as_str()), "acceptance item: {item}");
    }
}

// 3 — Some transcript renders its text; None renders the literal "não
// capturado" state and NOT an empty accordion body.
#[test]
fn captured_transcripts_render_their_text() {
    let html = render(&captured_vm()).into_string();
    assert!(html.contains("brief recebido · passo 1 leitura"), "task");
    assert!(html.contains("turn 1 system+charter"), "full");
    assert!(
        html.contains("conferi a re-emissão pra deslogado"),
        "journal"
    );
    // The honest sentinel must NOT appear when everything is captured.
    assert!(
        !html.contains(NOT_CAPTURED),
        "captured intent shows no not-captured state"
    );
}

#[test]
fn missing_transcripts_render_the_honest_state_not_a_blank() {
    let html = render(&uncaptured_vm()).into_string();
    // The explicit pt-BR not-captured sentence must appear once per missing
    // accordion (task + full + journal = 3).
    assert_eq!(
        html.matches(NOT_CAPTURED).count(),
        3,
        "all three missing accordions render the honest state"
    );
    // And the bodies must carry that text — never an empty <div class="ab">.
    assert!(
        !html.contains("class=\"ab\"></div>") && !html.contains("class=\"ab\"> </div>"),
        "no silent blank accordion body"
    );
}

// 4 — context_json content renders in the mono block; baixar/replay disabled.
#[test]
fn context_json_renders_in_mono_block_buttons_disabled() {
    let html = render(&captured_vm()).into_string();
    // The JSON is rendered into the mono block line-by-line; maud HTML-escapes
    // the quotes (`"` → `&quot;`), so assert on the escaped, faithful form.
    assert!(
        html.contains("&quot;schema_version&quot;: &quot;1.0.0&quot;"),
        "ctx content (escaped, in mono block)"
    );
    assert!(html.contains("a31.context.json"), "ctx filename header");
    // Both ⤓/▸ buttons disabled, with honest titles.
    assert!(html.contains("⤓ baixar context.json"), "baixar label");
    assert!(html.contains("▸ replay"), "replay label");
    assert!(
        html.contains("envelope ADR-0001 estiver capturado"),
        "honest baixar title"
    );
    assert!(
        html.contains("replay em wave futura"),
        "honest replay title"
    );
    // The disabled attribute must be present on the screen's buttons.
    assert!(html.contains("disabled"), "buttons are disabled");
}

// 5 — diff renders add/del lines with their classes; file rows with +N/-N.
#[test]
fn diff_renders_lines_with_classes_and_file_stats() {
    let html = render(&captured_vm()).into_string();
    assert!(
        html.contains("crates/auth/src/token.rs"),
        "diff file row path"
    );
    assert!(
        html.contains("+12") && html.contains("−4"),
        "file +/- stats"
    );
    // Add line carries the add (+hl) class and its text.
    assert!(
        html.contains("class=\"cl add hl\"") && html.contains("+     let iat = now();"),
        "add line + class"
    );
    // Del line carries the del class and its text.
    assert!(
        html.contains("class=\"cl del\"") && html.contains("-     let iat = old.iat;"),
        "del line + class"
    );
    // Context line is a bare .cl with the hunk header rendered.
    assert!(html.contains("@@ -40,5 +40,5 @@"), "hunk header");
}

// 6 — rail binds model, every principal in the chain, operator, metrics
// numbers, snapshot values, and each verdict outcome (adversarial marked).
#[test]
fn rail_binds_authorship_metrics_snapshot_verdicts() {
    let vm = captured_vm();
    let html = render(&vm).into_string();

    // Autoria.
    assert!(html.contains("opus-4.8"), "model");
    for p in &vm.authorship.principal_chain {
        assert!(html.contains(p.as_str()), "principal in chain: {p}");
    }
    assert!(html.contains("gustavo@humangr.com"), "operator");

    // Métricas — rendered verbatim from the VM.
    assert!(html.contains("38200"), "tokens");
    assert!(html.contains("877000"), "wall_ms");
    assert!(html.contains(">14<"), "tool_calls");
    assert!(html.contains("$0.04"), "cost");

    // Snapshot — values present (long tree hash truncated, full in title).
    assert!(html.contains("rustc 1.96.0"), "toolchain");
    assert!(html.contains("ws-9f2a"), "workspace");
    assert!(
        html.contains(
            "title=\"7e1abcdef0123456789abcdef0123456789abcdef0123456789abcdef01234567\""
        ),
        "full tree hash preserved in title"
    );
    assert!(
        html.contains("7e1abcdef01…"),
        "tree hash truncated for display"
    );

    // Verdicts — each outcome present; adversarial ones marked.
    assert!(html.contains("correctness"), "verdict reviewer 1");
    assert!(html.contains("security"), "verdict reviewer 2");
    assert!(html.contains("APPROVE"), "verdict outcome");
    assert!(html.contains("adversarial"), "adversarial verdict marked");
}

// 7 — `ver no PR` href correct (links the landing route).
#[test]
fn ver_no_pr_links_landing() {
    let html = render(&captured_vm()).into_string();
    assert!(html.contains("⤴ ver no PR #128"), "ver-no-PR action label");
    // The action is an <a> to the landing route.
    assert!(
        html.contains("<a class=\"btn\" href=\"/r/corelink-server/landing\">⤴ ver no PR #128</a>"),
        "ver-no-PR links the landing route"
    );
}

//! screen_repo_home — render oracle for WP-W3 (hand-built VM, no fixture.rs).
//!
//! Asserts faithfulness of every data-binding in `screens::repo_home::render`:
//!   1. file rows: name/message/age present; dir row vs file row (icon/class)
//!   2. intent_id row → real `/r/{repo}/intent/{id}` href; no-id row → no such link
//!   3. readme_html lands UN-escaped (HTML tag from it appears verbatim)
//!   4. About: description/topics/release/contributors bind; synergy lines render
//!   5. branch + branch_count bind

use hugit_web::provider::{AboutVm, RepoHomeVm, SynergyVm, TreeRowVm};
use hugit_web::screens::repo_home::render;

fn vm() -> RepoHomeVm {
    RepoHomeVm {
        repo: "myrepo".to_string(),
        branch: "feat/sessions".to_string(),
        branch_count: 7,
        files: vec![
            TreeRowVm {
                name: "crates".to_string(),
                is_dir: true,
                intent_id: Some("iABC".to_string()),
                message: "workspace root".to_string(),
                age: "há 5 min".to_string(),
            },
            TreeRowVm {
                name: "README.md".to_string(),
                is_dir: false,
                intent_id: None,
                message: "update readme".to_string(),
                age: "há 2 h".to_string(),
            },
        ],
        readme_html: "<h1>Hello World</h1><p>readme content</p>".to_string(),
        about: AboutVm {
            description: "the best forge ever".to_string(),
            topics: vec!["rust".to_string(), "cas".to_string()],
            release: Some("v1.2.3".to_string()),
            contributors: vec!["alice".to_string(), "bob".to_string()],
        },
        synergy: SynergyVm {
            lines: vec![
                (
                    "espelho GitHub".to_string(),
                    "sincronizado · 2 min".to_string(),
                ),
                ("intents abertos".to_string(), "42".to_string()),
            ],
        },
    }
}

/// 1a. File rows render name, message, age.
#[test]
fn file_row_renders_name_message_age() {
    let html = render(&vm()).into_string();
    assert!(html.contains("README.md"), "file name must appear");
    assert!(html.contains("update readme"), "file message must appear");
    assert!(html.contains("há 2 h"), "file age must appear");
}

/// 1b. Dir row vs file row: directory has the "▸" icon; file has "▤".
#[test]
fn dir_row_uses_branch_icon_file_uses_file_icon() {
    let html = render(&vm()).into_string();
    // Dir rows have "▸" and file rows have "▤"
    // Both appear in the output because we have one of each.
    assert!(
        html.contains("▸"),
        "dir icon ▸ must appear for directory row"
    );
    assert!(html.contains("▤"), "file icon ▤ must appear for file row");

    // Dir name is suffixed with "/"
    assert!(
        html.contains("crates/"),
        "directory name must be suffixed with /"
    );
    // Plain file name has no trailing slash
    assert!(
        !html.contains("README.md/"),
        "file name must NOT be suffixed with /"
    );
}

/// 2a. Row with intent_id renders the full intent href.
#[test]
fn row_with_intent_id_renders_intent_href() {
    let html = render(&vm()).into_string();
    assert!(
        html.contains("href=\"/r/myrepo/intent/iABC\""),
        "intent link href must be present: {}",
        &html[..html.len().min(1000)]
    );
    assert!(html.contains("iABC"), "intent id text must appear");
}

/// 2b. Row without intent_id renders no intent link for that row.
#[test]
fn row_without_intent_id_renders_no_intent_link() {
    // README.md row has no intent_id; confirm no link with README.md's slot.
    // We verify by checking there is exactly one intent link (for crates/).
    let html = render(&vm()).into_string();
    let count = html.matches("/r/myrepo/intent/").count();
    assert_eq!(
        count, 1,
        "exactly one intent link expected (for the dir row)"
    );
}

/// 3. readme_html lands UN-escaped: an HTML tag from it appears verbatim.
#[test]
fn readme_html_is_unescaped() {
    let html = render(&vm()).into_string();
    // If readme_html were escaped, "<h1>" would become "&lt;h1&gt;".
    assert!(
        html.contains("<h1>Hello World</h1>"),
        "readme_html must appear un-escaped in output"
    );
    assert!(
        !html.contains("&lt;h1&gt;"),
        "readme_html must NOT be HTML-escaped"
    );
}

/// 4a. About description and topics bind.
#[test]
fn about_description_and_topics_bind() {
    let html = render(&vm()).into_string();
    assert!(
        html.contains("the best forge ever"),
        "about description must appear"
    );
    assert!(html.contains("rust"), "topic 'rust' must appear");
    assert!(html.contains("cas"), "topic 'cas' must appear");
}

/// 4b. About release binds.
#[test]
fn about_release_binds() {
    let html = render(&vm()).into_string();
    assert!(html.contains("v1.2.3"), "release label must appear");
}

/// 4c. About contributors bind.
#[test]
fn about_contributors_bind() {
    let html = render(&vm()).into_string();
    assert!(html.contains("alice"), "contributor 'alice' must appear");
    assert!(html.contains("bob"), "contributor 'bob' must appear");
}

/// 4d. Synergy lines render (label + value).
#[test]
fn synergy_lines_render() {
    let html = render(&vm()).into_string();
    assert!(html.contains("espelho GitHub"), "synergy label must appear");
    assert!(
        html.contains("sincronizado · 2 min"),
        "synergy value must appear"
    );
    assert!(
        html.contains("intents abertos"),
        "second synergy label must appear"
    );
    assert!(html.contains("42"), "second synergy value must appear");
}

/// 5. Branch name and branch_count bind.
#[test]
fn branch_and_count_bind() {
    let html = render(&vm()).into_string();
    assert!(html.contains("feat/sessions"), "branch name must appear");
    assert!(html.contains("7"), "branch_count must appear");
}

/// Extra: no-release VM does not crash and omits release text.
#[test]
fn no_release_renders_cleanly() {
    let mut v = vm();
    v.about.release = None;
    let html = render(&v).into_string();
    // Should still render without panic; v1.2.3 must not appear
    assert!(
        !html.contains("v1.2.3"),
        "release must not appear when None"
    );
    // Core structure must still be present
    assert!(html.contains("About"), "About section must still render");
}

/// Extra: empty synergy renders the panel header but no rows.
#[test]
fn empty_synergy_renders_panel_header() {
    let mut v = vm();
    v.synergy.lines.clear();
    let html = render(&v).into_string();
    assert!(
        html.contains("camada hugit"),
        "synergy panel header must appear even when empty"
    );
    assert!(
        !html.contains("espelho GitHub"),
        "no synergy rows expected when empty"
    );
}

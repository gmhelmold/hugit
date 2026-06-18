//! `GET /v1/me/login` → [`LoginVm`].
//!
//! The login screen is the auth entry-point — it must be served WITHOUT a
//! Bearer token (there is no session yet). The route arm is a PRE-MATCH
//! if-guard in `server::route`, placed BEFORE the `match segs.as_slice()`
//! block (mirroring the `/readyz` guard) so it bypasses both the method gate
//! and the `two_tier_auth` call. No `EventLog` is involved: the response is
//! a fixed static card, transcribed byte-for-field from the canonical fixture
//! (`githugr-fixture/src/fixture.rs::w3b_login_vm`). No free-text is echoed
//! from any external source, so no scrubbing is needed.
//!
//! REAL: all seven fields are the canonical branded copy. There is no dynamic
//! data on the login card — it is purely presentational. This is honest: the
//! screen renders entirely from this VM (no chrome, no repo context).
//!
//! HONEST-DEFAULT: none applicable — there is no backing engine seam for this
//! screen at all; the static copy IS the correct response.

use hugit_http_contracts::LoginVm;

/// Build the login view-model.
///
/// Returns the canonical, static login card — all fields are fixed branded
/// copy transcribed from the githugr canonical fixture. No log parameter is
/// required (the login screen has no repo/identity context).
#[must_use]
pub fn build_login() -> LoginVm {
    LoginVm {
        // "Entrar no githugr" — the page heading.
        heading: "Entrar no githugr".to_string(),
        // The ONE accent button — GitHub social sign-in (also creates the account).
        github_label: "Continuar com GitHub".to_string(),
        // Secondary outline button — passkey.
        passkey_label: "Entrar com passkey".to_string(),
        // The quiet new-user note (canonical long form from the fixture).
        new_user_note: "Primeira vez? A conta nasce no primeiro login — e você importa seus repos do GitHub em um comando.".to_string(),
        // "importar →" — links to /import.
        import_label: "importar →".to_string(),
        // The honesty fine print (PAT / machine-token note).
        fine_print: "Uma conta HuGR pra família toda · tokens de máquina (PATs) são criados via CLI e nunca aparecem no navegador.".to_string(),
        // Footer left brand line.
        footer_note: "githugr · versionado por hugit".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── empty-log-equivalent (no log input; empty state is the only state) ────

    #[test]
    fn empty_log_honest_defaults() {
        // The login handler takes no log; calling it is the "empty" case.
        // Every field must be a non-empty canonical string (never a stub "").
        let vm = build_login();
        assert!(!vm.heading.is_empty(), "heading must be non-empty");
        assert!(
            !vm.github_label.is_empty(),
            "github_label must be non-empty"
        );
        assert!(
            !vm.passkey_label.is_empty(),
            "passkey_label must be non-empty"
        );
        assert!(
            !vm.new_user_note.is_empty(),
            "new_user_note must be non-empty"
        );
        assert!(
            !vm.import_label.is_empty(),
            "import_label must be non-empty"
        );
        assert!(!vm.fine_print.is_empty(), "fine_print must be non-empty");
        assert!(!vm.footer_note.is_empty(), "footer_note must be non-empty");
    }

    // ── populated / canonical-content tests ──────────────────────────────────

    #[test]
    fn canonical_heading() {
        assert_eq!(build_login().heading, "Entrar no githugr");
    }

    #[test]
    fn canonical_github_label() {
        assert_eq!(build_login().github_label, "Continuar com GitHub");
    }

    #[test]
    fn canonical_passkey_label() {
        assert_eq!(build_login().passkey_label, "Entrar com passkey");
    }

    #[test]
    fn canonical_new_user_note_long_form() {
        // The long form is the canonical fixture value (githugr-fixture w3b_login_vm).
        let note = build_login().new_user_note;
        assert!(
            note.contains("Primeira vez?"),
            "new_user_note must open with 'Primeira vez?'"
        );
        assert!(
            note.contains("em um comando"),
            "new_user_note must mention the import command"
        );
    }

    #[test]
    fn canonical_import_label() {
        assert_eq!(build_login().import_label, "importar →");
    }

    #[test]
    fn canonical_fine_print_mentions_pat() {
        let fp = build_login().fine_print;
        assert!(
            fp.contains("PAT") || fp.contains("tokens de máquina"),
            "fine_print must mention machine tokens / PATs"
        );
        assert!(
            fp.contains("nunca aparecem no navegador"),
            "fine_print must state tokens never appear in the browser"
        );
    }

    #[test]
    fn canonical_footer_note() {
        assert_eq!(build_login().footer_note, "githugr · versionado por hugit");
    }

    #[test]
    fn vm_round_trips() {
        let vm = build_login();
        let json = serde_json::to_string(&vm).expect("serialize must not fail");
        let reparsed: LoginVm =
            serde_json::from_str(&json).expect("round-trip must parse as LoginVm");
        assert_eq!(vm, reparsed, "LoginVm round-trip is lossless");
    }

    #[test]
    fn no_secret_shaped_content_in_output() {
        // The response is static copy — it must never contain anything that
        // looks like a token (the scrub detector pattern).
        let json = serde_json::to_string(&build_login()).unwrap();
        assert!(
            !json.contains("ghp_") && !json.contains("ghs_"),
            "login VM must not contain any PAT-shaped value"
        );
    }
}

//! `GET /v1/me/login` → `LoginVm`.
//! Transcribed BYTE-FOR-FIELD from the canonical source.

use serde::{Deserialize, Serialize};

/// The login card — no chrome: the screen owns its full document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginVm {
    pub heading: String,
    pub github_label: String,
    pub passkey_label: String,
    pub new_user_note: String,
    pub import_label: String,
    pub fine_print: String,
    pub footer_note: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_vm_round_trips() {
        let vm = LoginVm {
            heading: "Entrar no githugr".into(),
            github_label: "Continuar com GitHub".into(),
            passkey_label: "Entrar com passkey".into(),
            new_user_note: "Primeira vez? A conta nasce no primeiro login.".into(),
            import_label: "importar →".into(),
            fine_print: "Uma conta HuGR pra família toda.".into(),
            footer_note: "githugr · versionado por hugit".into(),
        };
        let json = serde_json::to_string(&vm).unwrap();
        let reparsed: LoginVm = serde_json::from_str(&json).unwrap();
        assert_eq!(vm, reparsed, "LoginVm round-trip is lossless");
    }
}

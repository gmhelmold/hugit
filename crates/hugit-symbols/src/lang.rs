//! Language detection — the single source of truth for extension→language.
//!
//! The serve/cli layers derive their display label (`lang` field on the wire VM)
//! from THIS table so the file the UI labels "rust" is the exact file the parser
//! parses (no drift between the display map and the parse map — master plan C1).

/// A source language the outliner can parse. Extension-derived, extensible:
/// add a variant + an arm in [`lang_for_ext`] + a query in `queries/` to grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
}

impl Lang {
    /// The stable display label the wire VM's `lang` field carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
        }
    }
}

/// Map a bare file extension (no leading dot, case-insensitive) to a [`Lang`].
///
/// The single source of truth shared by `hugit-symbols` and reused by the
/// serve/cli handlers for their display label. Unknown extensions → `None`
/// (the caller emits an empty outline — honest, never fabricated).
pub fn lang_for_ext(ext: &str) -> Option<Lang> {
    match ext.to_ascii_lowercase().as_str() {
        "rs" => Some(Lang::Rust),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_extension_maps_case_insensitively() {
        assert_eq!(lang_for_ext("rs"), Some(Lang::Rust));
        assert_eq!(lang_for_ext("RS"), Some(Lang::Rust));
        assert_eq!(lang_for_ext("Rs"), Some(Lang::Rust));
    }

    #[test]
    fn unknown_extension_is_none() {
        assert_eq!(lang_for_ext("py"), None);
        assert_eq!(lang_for_ext(""), None);
        assert_eq!(lang_for_ext("rust"), None); // bare ext only, not a name
    }

    #[test]
    fn display_label_is_stable() {
        assert_eq!(Lang::Rust.as_str(), "rust");
    }
}

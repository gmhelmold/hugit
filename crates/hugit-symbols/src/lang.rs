//! Language detection — the single source of truth for extension→language.
//!
//! The serve/cli layers derive their display label (`lang` field on the wire VM)
//! from THIS table so the file the UI labels "rust" is the exact file the parser
//! parses (no drift between the display map and the parse map — master plan C1).

/// A source language the outliner can parse. Extension-derived, extensible:
/// add a variant + an arm in [`lang_for_ext`] + a query in `queries/` + a
/// classifier arm in `outline_blob` to grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    /// TypeScript (`.ts`/`.mts`/`.cts`) — `LANGUAGE_TYPESCRIPT`.
    TypeScript,
    /// TSX (`.tsx`) — the JSX-enabled `LANGUAGE_TSX` of tree-sitter-typescript.
    Tsx,
    JavaScript,
    Python,
    Go,
    Java,
    C,
    /// C++ (`.cc`/`.cpp`/`.cxx`/`.hpp`/`.hh`/`.hxx`).
    Cpp,
    Ruby,
}

impl Lang {
    /// The stable display label the wire VM's `lang` field carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::TypeScript => "typescript",
            Lang::Tsx => "tsx",
            Lang::JavaScript => "javascript",
            Lang::Python => "python",
            Lang::Go => "go",
            Lang::Java => "java",
            Lang::C => "c",
            Lang::Cpp => "cpp",
            Lang::Ruby => "ruby",
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
        // TypeScript ships two grammars: plain TS and the JSX-enabled TSX.
        "ts" | "mts" | "cts" => Some(Lang::TypeScript),
        "tsx" => Some(Lang::Tsx),
        "js" | "jsx" | "mjs" | "cjs" => Some(Lang::JavaScript),
        "py" | "pyi" => Some(Lang::Python),
        "go" => Some(Lang::Go),
        "java" => Some(Lang::Java),
        "c" | "h" => Some(Lang::C),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Some(Lang::Cpp),
        "rb" => Some(Lang::Ruby),
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
    fn all_extensions_map() {
        for (ext, want) in [
            ("rs", Lang::Rust),
            ("ts", Lang::TypeScript),
            ("mts", Lang::TypeScript),
            ("cts", Lang::TypeScript),
            ("tsx", Lang::Tsx),
            ("js", Lang::JavaScript),
            ("jsx", Lang::JavaScript),
            ("mjs", Lang::JavaScript),
            ("cjs", Lang::JavaScript),
            ("py", Lang::Python),
            ("pyi", Lang::Python),
            ("go", Lang::Go),
            ("java", Lang::Java),
            ("c", Lang::C),
            ("h", Lang::C),
            ("cc", Lang::Cpp),
            ("cpp", Lang::Cpp),
            ("cxx", Lang::Cpp),
            ("hpp", Lang::Cpp),
            ("hh", Lang::Cpp),
            ("hxx", Lang::Cpp),
            ("rb", Lang::Ruby),
        ] {
            assert_eq!(lang_for_ext(ext), Some(want), "ext {ext}");
            assert_eq!(
                lang_for_ext(&ext.to_uppercase()),
                Some(want),
                "ext {ext} (upper)"
            );
        }
    }

    #[test]
    fn unknown_extension_is_none() {
        assert_eq!(lang_for_ext("txt"), None);
        assert_eq!(lang_for_ext(""), None);
        assert_eq!(lang_for_ext("rust"), None); // bare ext only, not a name
    }

    #[test]
    fn display_labels_are_stable() {
        assert_eq!(Lang::Rust.as_str(), "rust");
        assert_eq!(Lang::TypeScript.as_str(), "typescript");
        assert_eq!(Lang::Tsx.as_str(), "tsx");
        assert_eq!(Lang::JavaScript.as_str(), "javascript");
        assert_eq!(Lang::Python.as_str(), "python");
        assert_eq!(Lang::Go.as_str(), "go");
        assert_eq!(Lang::Java.as_str(), "java");
        assert_eq!(Lang::C.as_str(), "c");
        assert_eq!(Lang::Cpp.as_str(), "cpp");
        assert_eq!(Lang::Ruby.as_str(), "ruby");
    }
}

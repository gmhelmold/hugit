//! `claim-disjointness` — can two claims (sets of touched paths) land in
//! parallel lanes without a union re-test?
//!
//! Wraps the REAL union-engine primitive
//! [`hugit_queue::core::AffectedSet::is_disjoint`] — the exact disjointness law
//! the landing queue uses to decide whether two changes need a shared union
//! test. No second source of truth: the same `BTreeSet` intersection the engine
//! runs.
//!
//! ## Honest scope — the P2 path-approximation
//!
//! The union engine's disjointness is over *affected check-keys* — B3's true
//! blast radius (the memoized checks a change invalidates). In v1 hugit does not
//! yet compute that blast radius, so this tool treats each claim's **touched
//! file paths** AS the affected set. That is an APPROXIMATION:
//!
//! - It can report a FALSE OVERLAP (two claims touching the same file that the
//!   true check-graph would have proven independent) — conservative, safe (it
//!   over-serializes, never under-serializes).
//! - It can MISS a transitive overlap (claim A edits `a.rs`, claim B edits
//!   `b.rs`, but both feed the same downstream check) — the file-path view does
//!   not see that edge. So `disjoint: true` here is NECESSARY but not yet
//!   SUFFICIENT for a re-test-free parallel land; the union test remains the
//!   authority until B3's real affected-set seam lands.
//!
//! Every result states this caveat in `path_approximation` so a caller never
//! mistakes the approximation for the final word.

use serde_json::{Value, json};

use hugit_queue::core::AffectedSet;

use super::{ToolOutcome, req_str};

const PATH_APPROX_NOTE: &str = "v1 APPROXIMATION: disjointness is computed over touched FILE PATHS, \
    not the true memoized-check blast radius (B3). `disjoint: true` is necessary but NOT yet \
    sufficient for a re-test-free parallel land — two path-disjoint claims may still feed a shared \
    downstream check. The union test remains the authority. A `disjoint: false` (path overlap) is a \
    sound conservative serialize. The path-approximation is replaced by the real affected-set when \
    B3 lands.";

/// Extract a list of string paths from `args[key]`, requiring a non-empty array.
fn paths(args: &Value, key: &str) -> Result<Vec<String>, String> {
    let arr = args.get(key).and_then(Value::as_array).ok_or_else(|| {
        format!("missing or non-array required argument `{key}` (array of paths)")
    })?;
    if arr.is_empty() {
        return Err(format!("`{key}` must be a non-empty array of file paths"));
    }
    let mut out = Vec::with_capacity(arr.len());
    for (i, v) in arr.iter().enumerate() {
        let s = v
            .as_str()
            .ok_or_else(|| format!("`{key}[{i}]` is not a string"))?;
        if s.is_empty() {
            return Err(format!("`{key}[{i}]` is an empty path"));
        }
        out.push(s.to_string());
    }
    Ok(out)
}

/// Args: `{ "claim_a": { "id"?: str, "paths": [str] }, "claim_b": { ... } }`.
pub fn run(args: &Value) -> ToolOutcome {
    let a = match args.get("claim_a") {
        Some(v) => v,
        None => return ToolOutcome::err("missing required argument `claim_a`"),
    };
    let b = match args.get("claim_b") {
        Some(v) => v,
        None => return ToolOutcome::err("missing required argument `claim_b`"),
    };

    let a_paths = match paths(a, "paths") {
        Ok(p) => p,
        Err(e) => return ToolOutcome::err(format!("claim_a.{e}")),
    };
    let b_paths = match paths(b, "paths") {
        Ok(p) => p,
        Err(e) => return ToolOutcome::err(format!("claim_b.{e}")),
    };

    // Optional human labels (default to a stable placeholder).
    let a_id = req_str(a, "id").unwrap_or("claim_a");
    let b_id = req_str(b, "id").unwrap_or("claim_b");

    // The REAL engine primitive — same BTreeSet intersection the landing queue
    // runs. The keys ARE the file paths (the v1 approximation).
    let set_a = AffectedSet::new(a_paths.iter().cloned());
    let set_b = AffectedSet::new(b_paths.iter().cloned());
    let disjoint = set_a.is_disjoint(&set_b);

    // Report the overlapping keys (deterministic, sorted) for actionability.
    let overlap: Vec<&str> = set_a
        .keys()
        .filter(|k| set_b.keys().any(|other| other == *k))
        .collect();

    ToolOutcome::Ok(json!({
        "disjoint": disjoint,
        "claim_a": { "id": a_id, "affected_keys": set_a.keys().collect::<Vec<_>>() },
        "claim_b": { "id": b_id, "affected_keys": set_b.keys().collect::<Vec<_>>() },
        "overlap_keys": overlap,
        "engine": "hugit_queue::core::AffectedSet::is_disjoint",
        "path_approximation": PATH_APPROX_NOTE,
    }))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn disjoint_paths_report_disjoint() {
        let args = json!({
            "claim_a": { "id": "A", "paths": ["src/a.rs", "src/lib.rs"] },
            "claim_b": { "id": "B", "paths": ["src/b.rs", "src/main.rs"] }
        });
        let out = run(&args);
        match out {
            ToolOutcome::Ok(v) => {
                assert_eq!(v["disjoint"], json!(true));
                assert_eq!(v["overlap_keys"], json!([] as [&str; 0]));
                assert!(
                    v["path_approximation"]
                        .as_str()
                        .unwrap()
                        .contains("APPROXIMATION")
                );
            }
            ToolOutcome::Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn overlapping_paths_report_not_disjoint_and_name_the_overlap() {
        let args = json!({
            "claim_a": { "paths": ["src/a.rs", "src/shared.rs"] },
            "claim_b": { "paths": ["src/b.rs", "src/shared.rs"] }
        });
        match run(&args) {
            ToolOutcome::Ok(v) => {
                assert_eq!(v["disjoint"], json!(false));
                assert_eq!(v["overlap_keys"], json!(["src/shared.rs"]));
                // default ids when not supplied
                assert_eq!(v["claim_a"]["id"], json!("claim_a"));
            }
            ToolOutcome::Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn empty_paths_is_a_tool_error() {
        let args = json!({ "claim_a": { "paths": [] }, "claim_b": { "paths": ["x"] } });
        assert!(matches!(run(&args), ToolOutcome::Err(_)));
    }

    #[test]
    fn missing_claim_is_a_tool_error() {
        let args = json!({ "claim_a": { "paths": ["x"] } });
        assert!(matches!(run(&args), ToolOutcome::Err(_)));
    }

    #[test]
    fn non_string_path_is_a_tool_error() {
        let args = json!({ "claim_a": { "paths": [1, 2] }, "claim_b": { "paths": ["x"] } });
        assert!(matches!(run(&args), ToolOutcome::Err(_)));
    }
}

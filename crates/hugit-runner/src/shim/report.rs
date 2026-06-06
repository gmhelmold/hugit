//! Out-of-contract reporting for the Actions-YAML shim.
//!
//! ## Contract guarantee (②)
//! Any workflow construct outside the shim's published supported subset
//! produces an explicit [`OutOfContractReport`] that:
//! - names the unsupported construct precisely,
//! - gives an actionable remediation suggestion,
//! - is **never** silently skipped.
//!
//! Zero silent-skip code paths exist in this module or anywhere in the shim —
//! every unsupported construct MUST produce a report.

use std::fmt;

/// A single unsupported construct detected in a workflow, reported explicitly
/// and actionably.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedConstruct {
    /// The name of the unsupported construct (e.g., `"uses: actions/cache@v3"`).
    pub name: String,
    /// Actionable guidance: what to do instead or how to work around it.
    pub actionable_guidance: String,
}

/// An explicit, actionable report for one out-of-contract construct.
///
/// Never produced silently: every caller that detects an unsupported construct
/// must produce one of these. The field `actionable_guidance` must be
/// non-empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutOfContractReport {
    /// The construct that fell outside the supported subset.
    pub construct: UnsupportedConstruct,
}

impl OutOfContractReport {
    /// Construct an out-of-contract report for a named construct.
    ///
    /// `construct_name` must be non-empty; `guidance` must be non-empty and
    /// actionable (what the user can do to resolve it).
    pub fn unsupported(construct_name: &str, guidance: &str) -> Self {
        debug_assert!(
            !construct_name.is_empty(),
            "construct_name must be non-empty"
        );
        debug_assert!(
            !guidance.is_empty(),
            "actionable guidance must be non-empty"
        );
        Self {
            construct: UnsupportedConstruct {
                name: construct_name.to_string(),
                actionable_guidance: guidance.to_string(),
            },
        }
    }

    /// The name of the unsupported construct.
    pub fn construct_name(&self) -> &str {
        &self.construct.name
    }

    /// The actionable guidance for remediation.
    pub fn actionable_guidance(&self) -> &str {
        &self.construct.actionable_guidance
    }
}

impl fmt::Display for OutOfContractReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "OUT-OF-CONTRACT: `{}` — ACTION: {}",
            self.construct.name, self.construct.actionable_guidance
        )
    }
}

/// A diagnostic summary emitted by the shim after analyzing a workflow.
#[derive(Debug, Clone)]
pub struct ShimDiagnostic {
    /// All out-of-contract constructs detected (may be empty if workflow is
    /// fully within the supported subset).
    pub out_of_contract: Vec<OutOfContractReport>,
    /// Whether the workflow was fully within the supported subset.
    pub fully_supported: bool,
}

impl ShimDiagnostic {
    /// Build from a list of out-of-contract reports.
    pub fn from_reports(reports: Vec<OutOfContractReport>) -> Self {
        let fully_supported = reports.is_empty();
        Self {
            out_of_contract: reports,
            fully_supported,
        }
    }
}

impl fmt::Display for ShimDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.fully_supported {
            write!(f, "workflow is fully within the shim's supported subset")
        } else {
            write!(
                f,
                "{} out-of-contract construct(s) detected:",
                self.out_of_contract.len()
            )?;
            for r in &self.out_of_contract {
                write!(f, "\n  - {r}")?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_contract_report_has_nonempty_name_and_guidance() {
        let r = OutOfContractReport::unsupported(
            "uses: actions/cache@v3",
            "Remove the cache step or replace it with a run: step.",
        );
        assert!(!r.construct_name().is_empty());
        assert!(!r.actionable_guidance().is_empty());
    }

    #[test]
    fn out_of_contract_display_contains_action() {
        let r =
            OutOfContractReport::unsupported("uses: actions/cache@v3", "Remove the cache step.");
        let s = r.to_string();
        assert!(s.contains("OUT-OF-CONTRACT"));
        assert!(s.contains("ACTION"));
        assert!(s.contains("actions/cache@v3"));
    }

    #[test]
    fn shim_diagnostic_fully_supported_when_no_reports() {
        let d = ShimDiagnostic::from_reports(vec![]);
        assert!(d.fully_supported);
        assert!(d.out_of_contract.is_empty());
    }

    #[test]
    fn shim_diagnostic_not_fully_supported_with_reports() {
        let r = OutOfContractReport::unsupported("strategy:", "Remove the matrix strategy.");
        let d = ShimDiagnostic::from_reports(vec![r]);
        assert!(!d.fully_supported);
        assert_eq!(d.out_of_contract.len(), 1);
    }
}

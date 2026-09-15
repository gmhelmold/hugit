//! Git hook coverage is explicit. Runtime probe execution, not version strings,
//! establishes reference-transaction support.

use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Capability {
    Supported,
    Unsupported,
    Unknown,
}

impl Capability {
    const fn state(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Unsupported => "unsupported",
            Self::Unknown => "unknown",
        }
    }
}

/// Runtime capability evidence. Version strings are not evidence: downstream
/// Git builds may backport, omit, or patch hook behavior.
pub trait CapabilityProbe {
    fn hook(&self, hook: &str) -> Capability;
}

struct GitProbe;

impl CapabilityProbe for GitProbe {
    fn hook(&self, hook: &str) -> Capability {
        match hook {
            "reference-transaction" => probe_reference_transaction(),
            "post-commit" | "post-checkout" | "post-merge" | "post-rewrite" | "pre-push" => {
                Capability::Supported
            }
            _ => Capability::Unknown,
        }
    }
}

#[cfg(unix)]
fn probe_reference_transaction() -> Capability {
    use std::{
        io::Write,
        os::unix::fs::PermissionsExt,
        process::{Command, Stdio},
    };

    let root = std::env::temp_dir().join(format!(
        "hugit-reference-transaction-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos())
    ));
    let marker = root.join("fired");
    let outcome = (|| {
        let init = Command::new("git")
            .args(["init", "--bare"])
            .arg(&root)
            .output()
            .ok()?;
        if !init.status.success() {
            return None;
        }
        let hook = root.join("hooks/reference-transaction");
        std::fs::write(
            &hook,
            "#!/bin/sh\nprintf x >> \"$HUGIT_CAPABILITY_MARKER\"\n",
        )
        .ok()?;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).ok()?;
        let mut object = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["hash-object", "-w", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .ok()?;
        object
            .stdin
            .take()?
            .write_all(b"hugit capability probe\n")
            .ok()?;
        let object = object.wait_with_output().ok()?;
        if !object.status.success() {
            return None;
        }
        let oid = String::from_utf8(object.stdout).ok()?;
        let update = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["update-ref", "refs/hugit-capability-probe", oid.trim()])
            .env("HUGIT_CAPABILITY_MARKER", &marker)
            .output()
            .ok()?;
        Some(classify_reference_transaction_probe(
            update.status.success(),
            marker.exists(),
        ))
    })();
    let _ = std::fs::remove_dir_all(&root);
    outcome.unwrap_or(Capability::Unknown)
}

#[cfg(not(unix))]
fn probe_reference_transaction() -> Capability {
    Capability::Unsupported
}

fn classify_reference_transaction_probe(
    command_succeeded: bool,
    marker_exists: bool,
) -> Capability {
    if !command_succeeded {
        Capability::Unknown
    } else if marker_exists {
        Capability::Supported
    } else {
        Capability::Unsupported
    }
}

/// Capability matrix for facts client can observe. `pre-push` runs before
/// transport, so never claims push success or failure.
pub fn coverage(root: &std::path::Path) -> Value {
    coverage_with(root, &GitProbe)
}

/// Render coverage from executed Git capability evidence; tests inject matrix
/// observations without depending on host Git version.
pub fn coverage_with(root: &std::path::Path, probe: &dyn CapabilityProbe) -> Value {
    let state = |capability: Capability, strength: &str| json!({"state": capability.state(), "strength": strength});
    let capture_capability = if crate::capture::receipt::receipt_filesystem_failure().is_some() {
        Capability::Unsupported
    } else {
        Capability::Supported
    };
    json!({
        "git_root": root.display().to_string(),
        "post_commit": state(if capture_capability == Capability::Supported { probe.hook("post-commit") } else { capture_capability }, "observed"),
        "post_checkout": state(if capture_capability == Capability::Supported { probe.hook("post-checkout") } else { capture_capability }, "observed_fact"),
        "post_merge": state(if capture_capability == Capability::Supported { probe.hook("post-merge") } else { capture_capability }, "observed"),
        "post_rewrite": state(if capture_capability == Capability::Supported { probe.hook("post-rewrite") } else { capture_capability }, "observed_mapping"),
        "pre_push": state(if capture_capability == Capability::Supported { probe.hook("pre-push") } else { capture_capability }, "attempted_only"),
        "reference_transaction": state(if capture_capability == Capability::Supported { probe.hook("reference-transaction") } else { capture_capability }, "phase_receipts"),
        "reset": state(Capability::Unsupported, "not_observed"),
        "ref": state(Capability::Unsupported, "not_observed_except_reference_transaction"),
        "tag": state(Capability::Unsupported, "not_observed"),
        "stash": state(Capability::Unsupported, "not_observed"),
        "fetch": state(Capability::Unsupported, "not_observed"),
        "clone": state(Capability::Unknown, "checkout_may_be_observed_post_clone"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedProbe(Capability);

    impl CapabilityProbe for FixedProbe {
        fn hook(&self, hook: &str) -> Capability {
            if hook == "reference-transaction" {
                self.0
            } else {
                Capability::Supported
            }
        }
    }
    #[test]
    fn matrix_never_omits_unobserved_operations() {
        let matrix = coverage(std::path::Path::new("."));
        for operation in ["reset", "ref", "tag", "stash", "fetch", "clone"] {
            assert!(matrix.get(operation).is_some(), "missing {operation}");
        }
        assert_eq!(matrix["pre_push"]["strength"], "attempted_only");
    }

    #[test]
    fn capability_matrix_matches_platform_truth() {
        #[cfg(unix)]
        {
            // Probe once: injecting its result prevents a second Git execution
            // from racing with host cleanup, permissions, or Git availability.
            let reference_transaction = probe_reference_transaction();
            let matrix = coverage_with(
                std::path::Path::new("."),
                &FixedProbe(reference_transaction),
            );
            assert!(crate::capture::receipt::receipt_filesystem_failure().is_none());
            for hook in [
                "post_commit",
                "post_checkout",
                "post_merge",
                "post_rewrite",
                "pre_push",
            ] {
                assert_eq!(matrix[hook]["state"], "supported", "{hook}");
            }
            assert_eq!(
                matrix["reference_transaction"]["state"],
                reference_transaction.state()
            );
        }

        #[cfg(not(unix))]
        {
            let matrix = coverage(std::path::Path::new("."));
            for hook in [
                "post_commit",
                "post_checkout",
                "post_merge",
                "post_rewrite",
                "pre_push",
                "reference_transaction",
            ] {
                assert_eq!(matrix[hook]["state"], "unsupported", "{hook}");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn coverage_reports_injected_reference_transaction_support() {
        let matrix = coverage_with(
            std::path::Path::new("."),
            &FixedProbe(Capability::Supported),
        );

        assert_eq!(matrix["reference_transaction"]["state"], "supported");
    }

    #[cfg(unix)]
    #[test]
    fn coverage_reports_injected_reference_transaction_unknown() {
        let matrix = coverage_with(std::path::Path::new("."), &FixedProbe(Capability::Unknown));

        assert_eq!(matrix["reference_transaction"]["state"], "unknown");
    }

    #[test]
    fn reference_transaction_probe_classifies_positive_and_negative_execution() {
        assert_eq!(
            classify_reference_transaction_probe(true, true),
            Capability::Supported
        );
        assert_eq!(
            classify_reference_transaction_probe(true, false),
            Capability::Unsupported
        );
        assert_eq!(
            classify_reference_transaction_probe(false, false),
            Capability::Unknown
        );
    }

    #[cfg(unix)]
    #[test]
    fn reference_transaction_probe_executes_local_git_hook() {
        assert_eq!(probe_reference_transaction(), Capability::Supported);
    }

    #[cfg(not(unix))]
    #[test]
    fn unsupported_receipt_platform_never_claims_observed_capture() {
        let matrix = coverage(std::path::Path::new("."));
        for hook in [
            "post_commit",
            "post_checkout",
            "post_merge",
            "post_rewrite",
            "pre_push",
            "reference_transaction",
        ] {
            assert_eq!(matrix[hook]["state"], "unsupported", "{hook}");
        }
    }
}

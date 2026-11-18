use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

const CONFIG: &str = "examples/split_externs/parenox.edn";

struct TestDir(PathBuf);

impl TestDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("parenox-{name}-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn configured_paths_are_resolved_from_the_config_file_for_both_commands() {
    for command in ["check", "validate-externs"] {
        let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
            .args([command, "--config", CONFIG, "--json"])
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(output.status.success(), "{report}");
        assert_eq!(report["complete"], true);
        assert_eq!(report["files"].as_array().unwrap().len(), 5);
    }
}

#[test]
fn conventional_config_is_loaded_without_positional_paths() {
    let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .current_dir("examples/split_externs")
        .args(["check", "--json"])
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{report}");
    assert_eq!(report["files"][0]["path"], "app.clj");
}

#[test]
fn explicit_extern_flags_replace_the_configured_list() {
    let target = "examples/split_externs/drift.edn";
    let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args([
            "validate-externs",
            "--config",
            CONFIG,
            "--externs",
            target,
            "--json",
        ])
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(1));
    let files = report["files"].as_array().unwrap();
    assert_eq!(files.len(), 4);
    let externs = files.iter().find(|f| f["path"] == target).unwrap();
    let diagnostics = externs["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0]["code"], "PNX2322");
    assert_eq!(diagnostics[0]["contract"]["path"], target);
}

#[test]
fn invalid_configurations_report_the_failing_key_or_value() {
    let dir = TestDir::new("config-errors");
    let path = dir.0.join("parenox.edn");
    for (source, message) in [
        ("{\n :typo []}", "unknown configuration key :typo"),
        (
            "{:paths [] :paths []}",
            "duplicate configuration key :paths",
        ),
        ("{:paths \"src\"}", ":paths must be a vector of paths"),
        (
            "{:paths [#example \"src\"]}",
            ":paths requires non-empty path strings",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
            .args(["check", "--config"])
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains(message), "{error}");
        assert!(error.contains(&format!("{}:", path.display())), "{error}");
    }
}

#[test]
fn validation_does_not_exit_successfully_when_a_source_error_was_suppressed() {
    let dir = TestDir::new("incomplete-validation");
    fs::write(dir.0.join("lib.clj"), "(ns api)\n;; parenox-ignore PNX2322: legacy boundary\n(defn label {:t/sig '[:-> string?]} [] 42)").unwrap();
    fs::write(dir.0.join("api.edn"), "{api/label [:-> string?]}").unwrap();
    fs::write(
        dir.0.join("parenox.edn"),
        "{:paths [\"lib.clj\"] :externs [\"api.edn\"]}",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .current_dir(&dir.0)
        .args(["validate-externs", "--json"])
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(1), "{report}");
    assert_eq!(report["ok"], true);
    assert_eq!(report["complete"], false);
    assert_eq!(
        report["files"][0]["suppressed"].as_array().unwrap().len(),
        1
    );
}

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
fn incomplete_analysis_is_opt_in_and_keeps_definite_errors() {
    let dir = TestDir::new("coverage");
    fs::create_dir(dir.0.join("src")).unwrap();
    fs::write(dir.0.join("deps.edn"), "{:paths [\"src\"]}").unwrap();
    fs::write(
        dir.0.join("src/catalog.clj"),
        "(ns catalog) (defrecord Item [name]) (defn item [name] (->Item name))",
    )
    .unwrap();
    let caller = dir.0.join("src/shop.clj");
    let run = |extra: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
            .current_dir(&dir.0)
            .args(["check", "--json"])
            .args(extra)
            .output()
            .unwrap();
        (
            output.status.code(),
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        )
    };
    fs::write(
        &caller,
        "(ns shop (:require [catalog :as c])) (defn fetch [] (c/item \"one\"))",
    )
    .unwrap();
    let (strict, strict_report) = run(&[]);
    let (partial, partial_report) = run(&["--allow-incomplete"]);
    assert_eq!(strict, Some(1));
    assert_eq!(strict_report["ok"], false);
    assert_eq!(partial, Some(0));
    assert_eq!(partial_report["ok"], true);
    assert_eq!(partial_report["complete"], false);
    for file in partial_report["files"].as_array().unwrap() {
        assert_eq!(file["complete"], false);
        assert!(
            file["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .all(|d| d["severity"] == "warning")
        );
    }

    for (form, code) in [
        ("(defn fetch 42)", "PNX1001"),
        ("(defn fetch [] (c/item", "PNX9000"),
        ("(defn fetch {:t/sig '[:-> string?]} [] 42)", "PNX2322"),
        ("(definitely-not-a-known-macro fetch)", "PNX9001"),
    ] {
        fs::write(&caller, format!("(ns shop) {form}")).unwrap();
        let (status, report) = run(&["--allow-incomplete"]);
        assert_eq!(status, Some(1), "{report}");
        assert_eq!(report["ok"], false);
        let error = &report["files"][1]["diagnostics"][0];
        assert_eq!(error["code"], code, "{report}");
        assert_eq!(error["severity"], "error");
    }
}

#[test]
fn deps_edn_aliases_compose_project_source_roots() {
    let dir = TestDir::new("deps-paths");
    fs::write(
        dir.0.join("deps.edn"),
        r#"
      {:paths [:shared "resources"]
       :aliases {:shared ["lib"]
                 :test {:extra-paths [:tests]}
                 :tests ["test"]
                 :other {:replace-paths ["other"]}}}
    "#,
    )
    .unwrap();
    for (path, namespace) in [("lib", "lib"), ("test", "checks"), ("other", "other")] {
        fs::create_dir(dir.0.join(path)).unwrap();
        fs::write(
            dir.0.join(path).join("main.clj"),
            format!("(ns {namespace})"),
        )
        .unwrap();
    }
    let run = |aliases: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
            .current_dir(&dir.0)
            .args(["check", "--json"])
            .args(aliases)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    assert_eq!(run(&[])["path"], "lib/main.clj");
    assert_eq!(
        run(&["--alias", "test"])["files"][1]["path"],
        "test/main.clj"
    );
    let replaced = run(&["--alias", ":test", "--alias", "other"]);
    let paths: Vec<_> = replaced["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["other/main.clj", "test/main.clj"]);
}

#[test]
fn resolved_classpath_enables_navigation_to_local_dependency_source() {
    let dir = TestDir::new("classpath");
    fs::create_dir(dir.0.join("src")).unwrap();
    fs::create_dir(dir.0.join("library sources")).unwrap();
    fs::write(dir.0.join("deps.edn"), "{:paths [\"src\"]}").unwrap();
    let app = r#"(ns shop (:require [library.catalog :as lib]))
      (defn title {:t/sig '[:-> string?]} [] (lib/item))"#;
    let provider = "(ns library.catalog) (defn item {:t/sig '[:-> string?]} [] \"item\")";
    fs::write(dir.0.join("src/shop.clj"), app).unwrap();
    fs::write(dir.0.join("library sources/catalog.cljc"), provider).unwrap();
    fs::write(dir.0.join("dependency.jar"), b"compiled dependency").unwrap();
    let classpath = std::env::join_paths([
        "src",
        "library sources",
        "dependency.jar",
        "generated",
        "./src",
    ])
    .unwrap();
    fs::write(
        dir.0.join("classpath.txt"),
        format!("{}\r\n", classpath.to_str().unwrap()),
    )
    .unwrap();
    let lookup = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_parenox"))
            .current_dir(&dir.0)
            .args(["definition", "--file", "src/shop.clj", "--offset"])
            .arg(app.find("lib/item").unwrap().to_string())
            .args(extra)
            .output()
            .unwrap()
    };
    assert_eq!(lookup(&[]).status.code(), Some(1));
    let result = lookup(&["--classpath", "classpath.txt"]);
    assert!(result.status.success(), "{result:?}");
    let target: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(
        PathBuf::from(target["path"].as_str().unwrap()),
        PathBuf::from("library sources").join("catalog.cljc")
    );
    assert_eq!(
        target["span"]["start"]["offset"],
        provider.find("item").unwrap()
    );
    let report = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .current_dir(&dir.0)
        .args(["check", "--classpath", "classpath.txt", "--json"])
        .output()
        .unwrap();
    assert!(report.status.success(), "{report:?}");
    let report: Value = serde_json::from_slice(&report.stdout).unwrap();
    assert_eq!(report["files"].as_array().unwrap().len(), 2);
    assert_eq!(report["complete"], true);
}

#[test]
fn cyclic_deps_path_alias_is_reported() {
    let dir = TestDir::new("deps-cycle");
    fs::write(
        dir.0.join("deps.edn"),
        "{:paths [:a] :aliases {:a [:b] :b [:a]}}",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .current_dir(&dir.0)
        .arg("check")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("deps.edn:1:"), "{error}");
    assert!(
        error.contains("cyclic or too deeply nested path alias :a"),
        "{error}"
    );
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
fn definition_loads_test_paths_from_deps_edn() {
    let dir = TestDir::new("deps-definition");
    fs::write(
        dir.0.join("deps.edn"),
        r#"{:paths ["src"] :aliases {:test {:extra-paths ["test"]}}}"#,
    )
    .unwrap();
    for name in ["src", "test"] {
        fs::create_dir(dir.0.join(name)).unwrap();
    }
    let caller =
        "(ns app.test (:require [app.impl :as impl]))\n(defn check [] (impl/normalize \"é\"))";
    fs::write(dir.0.join("test/check.clj"), caller).unwrap();
    fs::write(
        dir.0.join("src/impl.clj"),
        "(ns app.impl)\n(defn normalize [value] value)",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .current_dir(&dir.0)
        .args([
            "definition",
            "--alias",
            "test",
            "--file",
            "test/check.clj",
            "--offset",
        ])
        .arg(caller.find("impl/normalize").unwrap().to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = serde_json::from_slice::<Value>(&output.stdout).unwrap();
    assert_eq!(result["path"], "src/impl.clj");
    assert_eq!(result["span"]["start"]["line"], 2);
    assert_eq!(result["span"]["start"]["column"], 7);
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

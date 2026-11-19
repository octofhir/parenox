use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::{Value, json};

const MANIFEST: &str = include_str!("fixtures/jvm-project/deps.edn");
const SOURCE: &str = include_str!("fixtures/jvm-project/src/demo/routing.cljc");
const TEST: &str = include_str!("fixtures/jvm-project/test/demo/routing_test.cljc");
const SOURCE_PATH: &str = "src/demo/routing.cljc";
const TEST_PATH: &str = "test/demo/routing_test.cljc";

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("parenox runtime {name} {}", std::process::id()));
        fs::create_dir(&path).unwrap();
        fs::create_dir_all(path.join("src/demo")).unwrap();
        fs::create_dir_all(path.join("test/demo")).unwrap();
        fs::write(path.join("deps.edn"), MANIFEST).unwrap();
        fs::write(path.join(SOURCE_PATH), SOURCE).unwrap();
        fs::write(path.join(TEST_PATH), TEST).unwrap();
        Self(path)
    }

    fn run(&self, selectors: &[&str], args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_parenox"))
            .arg("test")
            .arg("--project")
            .arg(&self.0)
            .args(["--alias", ":test"])
            .args(args)
            .args(selectors)
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn reports_a_missing_clojure_executable_as_a_launch_error() {
    let fixture = Fixture::new("missing executable");
    let missing = fixture.0.join("missing-clojure");
    let output = fixture.run(
        &["demo.routing-test"],
        &["--clojure", missing.to_str().unwrap(), "--format", "json"],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not start"));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert_eq!(report["run_error"]["phase"], "launch");
}

#[test]
fn rejects_unqualified_test_var_selectors() {
    let fixture = Fixture::new("invalid var selector");
    let output = fixture.run(&["--var", "route-status"], &[]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Expected a qualified Clojure test var")
    );
}

#[test]
fn discovery_reports_a_broken_namespace_before_starting_clojure() {
    let fixture = Fixture::new("broken discovered namespace");
    fs::write(
        fixture.0.join("test/demo/broken_test.cljc"),
        "(ns demo.broken-test [:oops})",
    )
    .unwrap();
    let output = fixture.run(
        &["--discover"],
        &[
            "--format",
            "json",
            "--clojure",
            "parenox-missing-executable",
        ],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let failure = &report["run_error"];
    assert_eq!(failure["phase"], "selection");
    assert!(
        std::path::Path::new(failure["path"].as_str().unwrap())
            .ends_with("test/demo/broken_test.cljc")
    );
    assert_eq!(failure["line"], 1);
    assert!(
        failure["message"]
            .as_str()
            .unwrap()
            .contains("closing delimiter")
    );
}

#[test]
#[ignore = "requires the Clojure CLI and JVM"]
fn jvm_locations_follow_assertion_helpers_and_type_methods() {
    let fixture = Fixture::new("shared assertions");
    let helper = "src/demo/location_helper.cljc";
    fs::write(
        fixture.0.join(helper),
        include_str!("fixtures/jvm-locations/helper.cljc"),
    )
    .unwrap();
    fs::write(
        fixture.0.join("test/demo/location_test.cljc"),
        include_str!("fixtures/jvm-locations/location_test.cljc"),
    )
    .unwrap();
    let output = fixture.run(&["demo.location-test"], &["--format", "json"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["tests"], 3);
    assert_eq!(result["failures"], 1);
    assert_eq!(result["errors"], 1);
    assert!(!fixture.0.join("worker-finished").exists());
    let findings = result["findings"].as_array().unwrap();
    for (kind, name, line) in [
        ("fail", "assertion-in-helper", 5),
        ("error", "error-in-type", 13),
    ] {
        let finding = findings
            .iter()
            .find(|finding| finding["kind"] == kind)
            .unwrap();
        assert_eq!(finding["test"], format!("demo.location-test/{name}"));
        assert_eq!(finding["path"], helper);
        assert_eq!(finding["line"], line);
        if kind == "error" {
            assert_eq!(
                finding["exception"]["class"],
                "java.lang.StringIndexOutOfBoundsException"
            );
            assert!(
                finding["exception"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("10")
            );
        } else {
            assert_eq!(finding["expected"], "(= :ready value)");
            let actual = finding["actual"].as_str().unwrap();
            assert!(actual.ends_with('…') && actual.chars().count() == 4097);
        }
    }
}

#[test]
#[ignore = "requires the Clojure CLI and JVM"]
fn project_jvm_pass_failure_and_load_error() {
    let fixture = Fixture::new("project");
    let run_ci = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_parenox"))
            .current_dir(&fixture.0)
            .args(args)
            .output()
            .unwrap()
    };
    let checked = run_ci(&[
        "check",
        "--alias",
        "test",
        "--allow-incomplete",
        "--format",
        "json",
    ]);
    assert_eq!(checked.status.code(), Some(0), "{checked:?}");
    let report: Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["complete"], false);

    let before = run_ci(&["export-externs"]);
    assert!(before.status.success(), "{before:?}");
    fs::write(fixture.0.join("before.edn"), before.stdout).unwrap();
    let narrowed = SOURCE.replace(":opt {:route", ":req {:route");
    fs::write(fixture.0.join(SOURCE_PATH), narrowed).unwrap();
    let after = run_ci(&["export-externs"]);
    assert!(after.status.success(), "{after:?}");
    fs::write(fixture.0.join("after.edn"), after.stdout).unwrap();
    let diff = run_ci(&[
        "diff-externs",
        "--before",
        "before.edn",
        "--after",
        "after.edn",
        "--json",
    ]);
    assert_eq!(diff.status.code(), Some(1), "{diff:?}");
    let changes: Value = serde_json::from_slice(&diff.stdout).unwrap();
    assert_eq!(changes["compatible"], false);
    assert_eq!(changes["complete"], true);
    let change = &changes["changes"][0];
    assert_eq!(change["name"], "demo.routing/status");
    assert_eq!(change["detail"]["reason"], "optional_field");
    assert_eq!(
        change["detail"]["path"][1],
        json!({"kind":"field","key":":route"})
    );
    fs::write(fixture.0.join(SOURCE_PATH), SOURCE).unwrap();

    let output = fixture.run(&["demo.routing-test"], &[]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("2 tests containing 3 assertions"));

    fs::write(
        fixture.0.join("test/demo/helper.clj"),
        "unread unrelated file",
    )
    .unwrap();
    let output = fixture.run(&["--discover"], &["--format", "json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["tests"], 2);
    assert_eq!(report["assertions"], 3);
    let output = run_ci(&["test", "--discover", "--format", "json"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["run_error"]["phase"], "selection");

    let edited = SOURCE.replace(
        "(if (some? (:route request)) :found :missing)",
        "(if (some? (:route request)) :found :found)",
    );
    fs::write(fixture.0.join(SOURCE_PATH), edited).unwrap();
    let output = fixture.run(&["demo.routing-test"], &["--format", "json"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let logs = String::from_utf8_lossy(&output.stderr);
    assert!(
        logs.contains(&format!(
            "{TEST_PATH}:7: fail in demo.routing-test/route-status"
        )),
        "{logs}"
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        result,
        json!({
            "version":1,"ok":false,"tests":2,"assertions":3,"failures":1,"errors":0,
            "findings":[{"kind":"fail","test":"demo.routing-test/route-status","path":TEST_PATH,"line":7,
                "expected":"(= :missing (routing/status {:route nil}))", "actual":"(not (= :missing :found))"}]
        })
    );

    let throwing = SOURCE.replace(
        "(if (some? (:route request)) :found :missing)",
        "(throw (ex-info \"route unavailable\" {}))",
    );
    fs::write(fixture.0.join(SOURCE_PATH), throwing).unwrap();
    let output = fixture.run(&["demo.routing-test"], &[]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!(
            "{SOURCE_PATH}:8: error in demo.routing-test/route-status"
        )),
        "{stdout}"
    );

    let output = fixture.run(&["demo.does-not-exist"], &["--format", "json"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["run_error"]["phase"], "load");
    assert!(result["run_error"]["path"].is_null());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Could not locate"));

    let bad_source = SOURCE.replace(":clj :jvm", ":clj (missing-function)");
    fs::write(fixture.0.join(SOURCE_PATH), bad_source).unwrap();
    let output = fixture.run(&["demo.routing-test"], &["--format", "json"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    let failure = &result["run_error"];
    assert_eq!(failure["phase"], "load");
    let path = std::path::Path::new(failure["path"].as_str().unwrap());
    assert!(path.ends_with(SOURCE_PATH), "{failure}");
    assert_eq!(failure["line"], 13);
    assert!(failure["column"].as_u64().is_some_and(|column| column > 0));
    assert!(
        failure["message"]
            .as_str()
            .unwrap()
            .contains("missing-function")
    );

    fs::write(
        fixture.0.join("test/demo/focused_test.clj"),
        include_str!("fixtures/focused_test.clj"),
    )
    .unwrap();
    let first = "demo.focused-test/passes";
    let second = "demo.focused-test/private-pass";
    let output = fixture.run(
        &["--var", first, "--var", first, "--var", second],
        &["--format", "json"],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["tests"], 2);
    assert_eq!(result["assertions"], 2);
    assert_eq!(
        String::from_utf8_lossy(&output.stderr)
            .matches("once fixture")
            .count(),
        1
    );

    let output = fixture.run(&["--var", "demo.focused-test/fails"], &["--format", "json"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["tests"], 1);
    assert_eq!(result["findings"][0]["test"], "demo.focused-test/fails");
    assert_eq!(result["findings"][0]["line"], 20);

    let output = fixture.run(
        &["--var", "demo.focused-test/no-test"],
        &["--format", "json"],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("Expected a clojure.test var"));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["run_error"]["phase"], "selection");
}

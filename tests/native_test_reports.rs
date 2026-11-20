use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const MANIFEST: &str = include_str!("fixtures/jvm-project/deps.edn");
const TEST_NAMESPACE: &str = "demo.portable-report-test";
const TEST_PATH: &str = "test/demo/portable_report_test.cljc";
const TEST: &str = include_str!("fixtures/portable-runtime/portable_report_test.cljc");
const PASSING: &str = include_str!("fixtures/jvm-project/test/demo/portable_test.cljc");
const MODERN_TEST: &str =
    include_str!("fixtures/portable-runtime/native-discovery/modern_test.cljc");
const LEGACY_TEST: &str =
    include_str!("fixtures/portable-runtime/native-discovery/legacy_test.cljrs");
const PRIVATE_TEST: &str = "portable.selected-test/secret";
const ERROR_TEST: &str = "portable.selected-test/errors";
const FAILED_TEST: &str = "portable.other-selected-test/fails";

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("parenox native test {name} {}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        fixture.write("deps.edn", MANIFEST);
        fixture.write(TEST_PATH, TEST);
        fixture.write("test/demo/portable_test.cljc", PASSING);
        fs::create_dir(fixture.0.join("empty")).unwrap();
        fixture
    }

    fn write(&self, path: &str, source: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }

    fn install_var_tests(&self) {
        self.write(
            "test/portable/selected_test.cljc",
            include_str!("fixtures/portable-runtime/test-reports/selected_test.cljc"),
        );
        self.write(
            "test/portable/other_selected_test.cljc",
            include_str!("fixtures/portable-runtime/test-reports/other_selected_test.cljc"),
        );
    }

    fn jvm_command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
        command
            .args(["test", "--project"])
            .arg(&self.0)
            .args(["--alias", "test"]);
        command
    }

    fn native_command(&self, binary: &Path) -> Command {
        self.native_command_with_roots(binary, &["test"])
    }

    fn native_command_with_roots(&self, binary: &Path, roots: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
        let empty = self.0.join("empty");
        command
            .env_clear()
            .env("PATH", &empty)
            .env("HOME", &empty)
            .current_dir(&empty)
            .args(["test", "--backend", "clojurust", "--project"])
            .arg(&self.0)
            .arg("--cljrs")
            .arg(binary);
        for root in roots {
            command.args(["--src-path", root]);
        }
        command
    }

    fn direct(&self, binary: &Path, roots: &[&str], namespaces: &[&str]) -> Output {
        let empty = self.0.join("empty");
        let mut command = Command::new(binary);
        command
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", &empty)
            .env("HOME", &empty)
            .args(["--no-config", "--no-native", "test"]);
        for root in roots {
            command.args(["--src-path", root]);
        }
        command.arg("--").args(namespaces).output().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn native_binary() -> PathBuf {
    let binary = std::env::var_os("PARENOX_CLJRS").expect("set PARENOX_CLJRS to a pinned cljrs");
    PathBuf::from(binary).canonicalize().unwrap()
}

fn log_lines(stdout: &[u8]) -> Vec<&str> {
    std::str::from_utf8(stdout)
        .unwrap()
        .lines()
        .map(|line| {
            if line.starts_with("Ran ") && line.ends_with("s.") {
                return line
                    .rsplit_once(" in ")
                    .map_or(line, |(summary, _)| summary);
            }
            line
        })
        .collect()
}

fn json_report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error}: {output:?}"))
}

fn test_finding<'a>(report: &'a Value, name: &str) -> &'a Value {
    let test = format!("{TEST_NAMESPACE}/{name}");
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["test"] == test)
        .unwrap_or_else(|| panic!("missing {test} in {report}"))
}

fn assert_incomplete(output: &Output, phase: &str, message: &str) {
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let report = json_report(output);
    assert_eq!(report["version"], 1);
    assert_eq!(report["ok"], false);
    for key in ["tests", "assertions", "failures", "errors"] {
        assert_eq!(report[key], 0, "{report}");
    }
    assert_eq!(report["findings"], json!([]), "{report}");
    assert_eq!(report["run_error"]["phase"], phase, "{report}");
    assert!(
        report["run_error"]["message"]
            .as_str()
            .unwrap()
            .contains(message),
        "{report}"
    );
}

fn test_vars(command: &mut Command, format: &str, tests: &[&str]) -> Output {
    command.args(["--format", format]);
    for test in tests {
        command.args(["--var", test]);
    }
    command.output().unwrap()
}

fn assert_selected_body_order(log: &str, expected: &[&str]) {
    let bodies: Vec<_> = log
        .lines()
        .filter(|line| line.starts_with("selected body "))
        .collect();
    assert_eq!(bodies, expected, "{log}");
    assert!(!log.contains("unexpected"), "{log}");
}

#[test]
#[ignore = "requires the pinned cljrs via PARENOX_CLJRS"]
fn native_test_preserves_the_external_runner_output_for_passing_and_missing_namespaces() {
    let fixture = Fixture::new("explicit namespaces");
    let binary = native_binary();
    let cases: &[(&[&str], i32, &str)] = &[
        (
            &["demo.portable-test"],
            0,
            "Ran 5 tests containing 10 assertions.",
        ),
        (
            &["demo.missing-test"],
            1,
            "Could not find namespace demo.missing-test on source path",
        ),
        (
            &["demo.portable-test", "demo.missing-test"],
            1,
            "Ran 5 tests containing 10 assertions across 2 namespace(s)",
        ),
    ];
    for &(namespaces, status, summary) in cases {
        let direct = fixture.direct(&binary, &["test"], namespaces);
        let wrapped = fixture
            .native_command(&binary)
            .args(namespaces)
            .output()
            .unwrap();
        assert_eq!(direct.status.code(), Some(status), "{direct:?}");
        assert_eq!(wrapped.status, direct.status, "{wrapped:?}");
        assert!(direct.stderr.is_empty(), "{direct:?}");
        assert_eq!(wrapped.stderr, direct.stderr, "{wrapped:?}");
        assert_eq!(log_lines(&wrapped.stdout), log_lines(&direct.stdout));
        let stdout = std::str::from_utf8(&wrapped.stdout).unwrap();
        assert!(stdout.contains(summary), "{stdout:?}");
    }
}

#[test]
fn native_test_rejects_unsupported_selectors_before_trying_to_launch() {
    let fixture = Fixture::new("unsupported selectors");
    let binary = fixture.0.join("missing-cljrs");
    let cases: &[(&[&str], &str)] = &[
        (
            &["--var", "demo.portable-test"],
            "Expected a qualified Clojure test var",
        ),
        (&["--alias", "test", TEST_NAMESPACE], "--alias"),
        (
            &["--clojure", "unused-clojure", TEST_NAMESPACE],
            "--clojure",
        ),
        (
            &[TEST_NAMESPACE, "demo/portable-test"],
            "Expected a Clojure namespace",
        ),
        (&[], "<NAMESPACE>"),
    ];
    for (args, expected) in cases {
        let output = fixture
            .native_command(&binary)
            .args(*args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        assert!(stderr.contains(expected), "{args:?}: {stderr}");
        assert!(!stderr.contains("could not start"), "{args:?}: {stderr}");
    }
}

#[test]
fn native_json_test_reports_selection_and_launcher_failures() {
    let fixture = Fixture::new("JSON prelaunch");
    let binary = fixture.0.join("missing-cljrs");
    let cases: &[(&[&str], &str, &str)] = &[
        (&[TEST_NAMESPACE], "launch", "could not start"),
        (&["--var", PRIVATE_TEST], "launch", "could not start"),
        (
            &["--var", "demo.portable-test/with/multiple/slashes"],
            "selection",
            "Expected a qualified Clojure test var",
        ),
        (&["--alias", "test", TEST_NAMESPACE], "selection", "--alias"),
        (
            &["demo/malformed"],
            "selection",
            "Expected a Clojure namespace",
        ),
    ];
    for &(args, phase, message) in cases {
        let output = fixture
            .native_command(&binary)
            .args(["--format", "json"])
            .args(args)
            .output()
            .unwrap();
        assert_incomplete(&output, phase, message);
    }
}

#[test]
#[ignore = "requires the pinned cljrs via PARENOX_CLJRS"]
fn native_test_discovers_conventional_namespaces_using_active_reader_branches() {
    let fixture = Fixture::new("discovery branches");
    let binary = native_binary();
    for (path, source) in [
        ("main/discovery/modern_test.cljc", MODERN_TEST),
        ("other/discovery/modern_test.cljc", MODERN_TEST),
        ("other/discovery/legacy_test.cljrs", LEGACY_TEST),
        ("main/.hidden/invalid_test.cljc", "42"),
        ("main/discovery/.hidden_test.cljc", "42"),
        ("main/discovery/jvm_test.clj", "42"),
        ("main/discovery/helper.cljrs", "42"),
    ] {
        fixture.write(path, source);
    }

    let roots = &["main", "other", "main"];
    let wrapped = fixture
        .native_command_with_roots(&binary, roots)
        .arg("--discover")
        .output()
        .unwrap();
    let direct = fixture.direct(
        &binary,
        roots,
        &["discovery.legacy-test", "discovery.modern-test"],
    );
    assert_eq!(direct.status.code(), Some(0), "{direct:?}");
    assert_eq!(wrapped.status, direct.status, "{wrapped:?}");
    assert!(direct.stderr.is_empty(), "{direct:?}");
    assert_eq!(wrapped.stderr, direct.stderr, "{wrapped:?}");
    let wrapped_lines = log_lines(&wrapped.stdout);
    assert_eq!(wrapped_lines, log_lines(&direct.stdout));
    assert!(wrapped_lines.contains(&"Ran 2 tests containing 2 assertions across 2 namespace(s)"));

    let structured = fixture
        .native_command_with_roots(&binary, roots)
        .args(["--format", "json", "--discover"])
        .output()
        .unwrap();
    assert_eq!(structured.status.code(), Some(0), "{structured:?}");
    assert_eq!(
        json_report(&structured),
        json!({
            "version": 1, "ok": true, "tests": 2, "assertions": 2,
            "failures": 0, "errors": 0, "findings": [],
        })
    );
    let log = std::str::from_utf8(&structured.stderr).unwrap();
    for name in ["discovery.legacy-test", "discovery.modern-test"] {
        assert!(log.contains(&format!("Testing {name}")), "{log}");
    }
}

#[test]
fn native_test_discovery_requires_roots_and_test_namespaces_before_launch() {
    let fixture = Fixture::new("discovery errors");
    let binary = fixture.0.join("missing-cljrs");
    fixture.write(
        "broken/no_namespace_test.cljc",
        "#?(:clj (ns broken.no-namespace-test))\n",
    );
    let cases: &[(&[&str], &str, Option<&str>)] = &[
        (&[], "--src-path", None),
        (&["empty"], "No *_test", None),
        (
            &["broken"],
            "Expected a namespace declaration",
            Some("broken/no_namespace_test.cljc"),
        ),
    ];
    for &(roots, expected, path) in cases {
        let output = fixture
            .native_command_with_roots(&binary, roots)
            .arg("--discover")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{roots:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{roots:?}: {output:?}");
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        assert!(stderr.contains(expected), "{roots:?}: {stderr}");
        assert!(!stderr.contains("could not start"), "{roots:?}: {stderr}");
        if let Some(path) = path {
            let path = Path::new(path).components().collect::<PathBuf>();
            assert!(
                stderr.contains(path.to_str().unwrap()),
                "{roots:?}: {stderr}"
            );
        }
    }
}

#[test]
#[ignore = "requires the pinned cljrs via PARENOX_CLJRS"]
fn native_json_test_distinguishes_namespace_load_failure_from_zero_tests() {
    let fixture = Fixture::new("JSON load vs no tests");
    let binary = native_binary();
    fixture.write(
        "test/demo/unbound_test.cljc",
        "(ns demo.unbound-test)\nmissing-load-var\n",
    );
    fixture.write(
        "test/demo/no_tests.cljc",
        "(ns demo.no-tests)\n(def helper 42)\n",
    );
    for (namespace, phase, message) in [
        ("demo.missing-test", "load", "demo.missing-test"),
        ("demo.unbound-test", "load", "missing-load-var"),
        ("demo.no-tests", "execution", "no clojure.test vars"),
    ] {
        let output = fixture
            .native_command(&binary)
            .args(["--format", "json", namespace])
            .output()
            .unwrap();
        assert_incomplete(&output, phase, message);
    }

    let partly_missing = fixture
        .native_command(&binary)
        .args(["--format", "json", TEST_NAMESPACE, "demo.missing-test"])
        .output()
        .unwrap();
    assert_incomplete(&partly_missing, "load", "demo.missing-test");
    let log = std::str::from_utf8(&partly_missing.stderr).unwrap();
    assert!(!log.contains("portable test output"), "{log}");
}

#[test]
#[cfg(unix)]
fn native_test_keeps_the_external_executables_streams_and_exit_status() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new("process streams");
    let executable = fixture.0.join("cljrs stub");
    fs::write(
        &executable,
        concat!(
            "#!/bin/sh\n",
            "[ -f test/demo/portable_report_test.cljc ] || exit 34\n",
            "printf 'portable stdout\\n'\n",
            "printf 'portable stderr\\n' >&2\n",
            "exit 17\n",
        ),
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let output = fixture
        .native_command(&executable)
        .args(["--format", "text"])
        .arg(TEST_NAMESPACE)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(17), "{output:?}");
    assert_eq!(output.stdout, b"portable stdout\n");
    assert_eq!(output.stderr, b"portable stderr\n");

    let structured = fixture
        .native_command(&executable)
        .args(["--format", "json", TEST_NAMESPACE])
        .output()
        .unwrap();
    assert_incomplete(&structured, "launch", "did not start the test runner");
    let log = std::str::from_utf8(&structured.stderr).unwrap();
    assert!(log.contains("portable stdout\n"), "{log}");
    assert!(log.contains("portable stderr\n"), "{log}");
}

#[test]
#[ignore = "requires the Clojure CLI, JVM and the pinned cljrs via PARENOX_CLJRS"]
fn runtime_test_var_selection_orders_groups_and_runs_each_selected_var_once() {
    let fixture = Fixture::new("selected vars");
    fixture.install_var_tests();
    let binary = native_binary();
    let selectors = [PRIVATE_TEST, FAILED_TEST, ERROR_TEST, PRIVATE_TEST];
    let jvm = test_vars(&mut fixture.jvm_command(), "json", &selectors);
    let native = test_vars(&mut fixture.native_command(&binary), "json", &selectors);
    let jvm_report = json_report(&jvm);
    let native_report = json_report(&native);
    let ordered_bodies = [
        "selected body private",
        "selected body error",
        "selected body failure",
    ];
    for (output, report) in [(&jvm, &jvm_report), (&native, &native_report)] {
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(report["version"], 1);
        assert_eq!(report["ok"], false);
        assert!(report.get("run_error").is_none(), "{report}");
        assert_eq!(report["tests"], 3);
        assert_eq!(report["assertions"], 3);
        assert_eq!(report["failures"], 1);
        assert_eq!(report["errors"], 1);
        let findings = report["findings"].as_array().unwrap();
        assert_eq!(findings.len(), 2, "{report}");
        assert_eq!(findings[0]["test"], ERROR_TEST);
        assert_eq!(findings[0]["kind"], "error");
        assert_eq!(findings[1]["test"], FAILED_TEST);
        assert_eq!(findings[1]["kind"], "fail");
        assert_eq!(findings[1]["message"], "selected assertion mismatch");
        assert_eq!(findings[1]["expected"], "(= :expected :actual)");
        assert_selected_body_order(
            std::str::from_utf8(&output.stderr).unwrap(),
            &ordered_bodies,
        );
    }
    assert_eq!(
        jvm_report["findings"][0]["exception"]["message"],
        "selected error Δ"
    );
    assert_eq!(native_report["findings"][0]["actual"], "selected error Δ");

    for (selection, status, summary, bodies) in [
        (
            &[PRIVATE_TEST][..],
            0,
            "Ran 1 tests containing 1 assertions.",
            &["selected body private"][..],
        ),
        (
            &selectors[..],
            1,
            "Ran 3 tests containing 3 assertions.",
            &ordered_bodies[..],
        ),
    ] {
        let output = test_vars(&mut fixture.native_command(&binary), "text", selection);
        assert_eq!(output.status.code(), Some(status), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let log = std::str::from_utf8(&output.stdout).unwrap();
        assert!(log.contains(summary), "{log}");
        assert_selected_body_order(log, bodies);
    }
}

#[test]
#[ignore = "requires the pinned cljrs via PARENOX_CLJRS"]
fn native_test_var_selection_validates_all_names_before_running_test_bodies() {
    let fixture = Fixture::new("invalid selected vars");
    fixture.install_var_tests();
    let binary = native_binary();
    for missing in [
        "portable.selected-test/missing",
        "portable.selected-test/ordinary",
        "portable.selected-test/borrowed",
    ] {
        let failure = test_vars(
            &mut fixture.native_command(&binary),
            "json",
            &[PRIVATE_TEST, missing],
        );
        let message = format!("Expected a clojure.test var: {missing}");
        assert_incomplete(&failure, "selection", &message);
        assert_selected_body_order(std::str::from_utf8(&failure.stderr).unwrap(), &[]);
    }

    let failure = test_vars(
        &mut fixture.native_command(&binary),
        "json",
        &[
            "portable.selected-test/missing",
            "portable.missing-test/test-var",
        ],
    );
    assert_incomplete(&failure, "load", "portable.missing-test");
    assert_selected_body_order(std::str::from_utf8(&failure.stderr).unwrap(), &[]);

    let text = test_vars(
        &mut fixture.native_command(&binary),
        "text",
        &["portable.selected-test/ordinary"],
    );
    assert_eq!(text.status.code(), Some(2), "{text:?}");
    assert!(text.stdout.is_empty(), "{text:?}");
    assert!(
        std::str::from_utf8(&text.stderr)
            .unwrap()
            .contains("Expected a clojure.test var: portable.selected-test/ordinary"),
        "{text:?}"
    );
}

#[test]
#[ignore = "requires the Clojure CLI, JVM and the pinned cljrs via PARENOX_CLJRS"]
fn portable_test_reports_count_failures_and_exceptions_on_both_runtimes() {
    let fixture = Fixture::new("mixed reports");
    let jvm = fixture
        .jvm_command()
        .args(["--format", "json", TEST_NAMESPACE])
        .output()
        .unwrap();

    let binary = native_binary();
    let native = fixture
        .native_command(&binary)
        .arg(TEST_NAMESPACE)
        .output()
        .unwrap();
    let direct = fixture.direct(&binary, &["test"], &[TEST_NAMESPACE]);
    let mut structured = fixture.native_command(&binary);
    #[cfg(unix)]
    {
        let temporary = fixture.0.join(r#"temporary "quoted"\path"#);
        fs::create_dir(&temporary).unwrap();
        structured.env("TMPDIR", temporary);
    }
    let structured = structured
        .args(["--format", "json", TEST_NAMESPACE, TEST_NAMESPACE])
        .output()
        .unwrap();

    assert_eq!(jvm.status.code(), Some(1), "{jvm:?}");
    assert_eq!(native.status.code(), jvm.status.code(), "{native:?}");
    assert_eq!(
        structured.status.code(),
        jvm.status.code(),
        "{structured:?}"
    );
    assert_eq!(native.status, direct.status, "{direct:?}");
    assert_eq!(native.stderr, direct.stderr, "{direct:?}");
    // Test vars are reported in a different order across cljrs processes.
    let mut native_lines = log_lines(&native.stdout);
    let mut direct_lines = log_lines(&direct.stdout);
    native_lines.sort_unstable();
    direct_lines.sort_unstable();
    assert_eq!(native_lines, direct_lines);
    let jvm_report = json_report(&jvm);
    let native_report = json_report(&structured);
    for report in [&jvm_report, &native_report] {
        assert_eq!(report["version"], 1);
        assert_eq!(report["ok"], false);
        assert!(report.get("run_error").is_none(), "{report}");
        assert_eq!(report["tests"], 4);
        assert_eq!(report["assertions"], 4);
        assert_eq!(report["failures"], 1);
        assert_eq!(report["errors"], 2);
        assert_eq!(report["findings"].as_array().unwrap().len(), 3, "{report}");
    }
    for finding in native_report["findings"].as_array().unwrap() {
        for native_unavailable in ["path", "line", "exception"] {
            assert!(finding.get(native_unavailable).is_none(), "{finding}");
        }
    }
    let failure = test_finding(&jvm_report, "incorrect-result");
    assert_eq!(failure["kind"], "fail");
    assert_eq!(failure["path"], TEST_PATH);
    assert_eq!(failure["actual"], "(not (= :expected :actual))");
    let native_failure = test_finding(&native_report, "incorrect-result");
    for key in ["kind", "message", "expected"] {
        assert_eq!(native_failure[key], failure[key]);
    }
    assert_eq!(
        native_failure["actual"],
        "(clojure.core/not (= :expected :actual))"
    );
    for (name, message) in [
        ("exception-in-assertion", "portable assertion exception"),
        ("exception-in-test-body", "portable test exception"),
    ] {
        let error = test_finding(&jvm_report, name);
        assert_eq!(error["kind"], "error");
        assert_eq!(error["path"], TEST_PATH);
        assert_eq!(
            error["exception"],
            json!({"class":"clojure.lang.ExceptionInfo","message":message})
        );
        let native_error = test_finding(&native_report, name);
        assert_eq!(native_error["kind"], error["kind"]);
        assert_eq!(native_error["actual"], message);
    }

    let jvm_log = String::from_utf8(jvm.stderr).unwrap();
    let native_log = String::from_utf8(native.stdout).unwrap();
    let structured_log = String::from_utf8(structured.stderr).unwrap();
    assert!(native.stderr.is_empty(), "{:?}", native.stderr);
    for log in [&jvm_log, &native_log, &structured_log] {
        for expected in [
            "Testing demo.portable-report-test",
            "portable test output",
            "portable assertion mismatch",
            "portable assertion exception",
            "portable test exception",
            "Ran 4 tests containing 4 assertions.",
            "1 failures, 2 errors.",
        ] {
            assert!(log.contains(expected), "missing {expected:?} in {log:?}");
        }
    }
    for expected in [
        "ERROR in #'demo.portable-report-test/exception-in-assertion",
        "ERROR in #'demo.portable-report-test/exception-in-test-body",
        "FAIL in #'demo.portable-report-test/incorrect-result",
        "(clojure.core/not (= :expected :actual))",
        "#error {:cause portable test exception, :data {:code :boom}",
        "1 passed, 1 failed, 2 errors.",
    ] {
        assert!(
            native_log.contains(expected),
            "missing {expected:?} in {native_log:?}"
        );
    }
    assert!(!native_log.contains("portable_report_test.cljc"));
}

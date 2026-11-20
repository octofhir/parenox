use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

const MANIFEST: &str = include_str!("fixtures/jvm-project/deps.edn");
const SOURCE: &str = include_str!("fixtures/jvm-project/src/demo/routing.cljc");
const TEST: &str = include_str!("fixtures/jvm-project/test/demo/routing_test.cljc");
const SOURCE_PATH: &str = "src/demo/routing.cljc";
const TEST_PATH: &str = "test/demo/routing_test.cljc";
const SCRIPT: &str = include_str!("fixtures/jvm-project/script.clj");

#[cfg(unix)]
const TIMEOUT_STUB: &str = r#"#!/bin/sh
printf 'runtime stdout\n'
printf 'runtime stderr\n' >&2
printf '%s' "$$" > child.pid
( printf 'descendant started\n' >&2; /bin/sleep 6; : > descendant-survived ) &
wait
"#;

#[cfg(unix)]
const CANCELLATION_STUB: &str = r#"#!/bin/sh
printf 'runtime stdout\n'
printf 'runtime stderr\n' >&2
printf '%s' "$$" > child.pid
: > child.ready
exec /bin/sleep 10
"#;

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
        self.test_command()
            .args(["--alias", ":test"])
            .args(args)
            .args(selectors)
            .output()
            .unwrap()
    }

    fn test_command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
        command.args(["test", "--project"]).arg(&self.0);
        command
    }

    fn eval(&self, args: &[&str]) -> Output {
        self.eval_command().args(args).output().unwrap()
    }

    fn eval_command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
        command
            .args(["eval", "--format", "json", "--project"])
            .arg(&self.0);
        command
    }

    fn app(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
        command.args(["run", "--project"]).arg(&self.0);
        command
    }

    #[cfg(unix)]
    fn launcher(&self, name: &str, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = self.0.join(name);
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    fn signal_after_runtime_starts(
        &self,
        child: std::process::Child,
        signal: libc::c_int,
    ) -> Output {
        use std::thread;
        use std::time::{Duration, Instant};

        let ready = self.0.join("child.ready");
        let deadline = Instant::now() + Duration::from_secs(6);
        while !ready.is_file() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        if !ready.is_file() {
            panic!(
                "runtime launcher did not start: {:?}",
                child.wait_with_output()
            );
        }
        // Signal the Parenox process spawned by this test.
        let sent = unsafe { libc::kill(child.id() as libc::pid_t, signal) };
        let output = child.wait_with_output().unwrap();
        assert_eq!(sent, 0, "{output:?}");
        output
    }

    #[cfg(unix)]
    fn assert_child_reaped(&self, output: &Output) {
        let pid = fs::read_to_string(self.0.join("child.pid"))
            .unwrap_or_else(|error| {
                panic!("runtime launcher did not record its PID: {error}; {output:?}")
            })
            .parse::<libc::pid_t>()
            .unwrap();
        // Signal 0 checks for the recorded PID without delivering a signal.
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "child {pid} still exists"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    fn install_app(&self) {
        for (path, contents) in [
            (
                "app/demo/app.clj",
                include_str!("fixtures/jvm-project/app/demo/app.clj"),
            ),
            (
                "greeting/deps.edn",
                include_str!("fixtures/jvm-project/greeting/deps.edn"),
            ),
            (
                "greeting/src/demo/greeting.clj",
                include_str!("fixtures/jvm-project/greeting/src/demo/greeting.clj"),
            ),
            ("script.clj", SCRIPT),
        ] {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn eval_reports_a_missing_clojure_executable() {
    let fixture = Fixture::new("eval missing executable");
    let executable = fixture.0.join("missing-clojure");
    let output = fixture.eval(&["--expr", "42", "--clojure", executable.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], false);
    assert_eq!(result["run_error"]["phase"], "launch");
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not start"));
}

#[cfg(unix)]
#[test]
fn eval_times_out_and_stops_the_runtime_group() {
    for (backend, options) in [
        ("jvm", &["--clojure"][..]),
        ("clojurust", &["--backend", "clojurust", "--cljrs"][..]),
    ] {
        let fixture = Fixture::new(&format!("eval timeout {backend}"));
        let executable = fixture.launcher("runtime stub", TIMEOUT_STUB);
        let output = fixture
            .eval_command()
            .args(["--expr", "42", "--timeout-ms", "3000"])
            .args(options)
            .arg(executable)
            .stdin(Stdio::null())
            .output()
            .unwrap();

        assert_eval_interrupted(&output, 124, "Clojure eval timed out after 3000 ms");
        fixture.assert_child_reaped(&output);
        assert!(String::from_utf8_lossy(&output.stderr).contains("descendant started"));
        // The descendant inherits the captured streams, so output() also
        // waits for it to release them if process-group cancellation fails.
        assert!(!fixture.0.join("descendant-survived").exists(), "{backend}");
    }
}

#[cfg(unix)]
#[test]
fn eval_cancels_a_supervised_runtime_on_sigterm() {
    let fixture = Fixture::new("eval cancellation");
    let executable = fixture.launcher("clojure stub", CANCELLATION_STUB);
    let child = fixture
        .eval_command()
        .args(["--expr", "42", "--timeout-ms", "8000", "--clojure"])
        .arg(executable)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = fixture.signal_after_runtime_starts(child, libc::SIGTERM);
    assert_eval_interrupted(
        &output,
        143,
        &format!("Clojure eval cancelled by signal {}", libc::SIGTERM),
    );
    fixture.assert_child_reaped(&output);
}

#[cfg(unix)]
fn assert_eval_interrupted(output: &Output, code: i32, message: &str) {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        result,
        json!({
            "version": 1,
            "ok": false,
            "run_error": {"phase": "execution", "message": message}
        })
    );
    let log = std::str::from_utf8(&output.stderr).unwrap();
    for expected in ["runtime stdout\n", "runtime stderr\n", message] {
        assert!(log.contains(expected), "{log:?}");
    }
}

#[cfg(unix)]
#[test]
fn eval_decodes_a_completed_supervised_run() {
    let fixture = Fixture::new("eval timed success");
    let executable = fixture.launcher(
        "clojure stub",
        r#"#!/bin/sh
printf 'runtime stdout\n'
printf 'runtime stderr\n' >&2
printf '%s' '[:value "42"]' > "$5"
"#,
    );
    for timeout in [&[][..], &["--timeout-ms", "6000"][..]] {
        let output = fixture
            .eval_command()
            .args(["--expr", "42", "--clojure"])
            .arg(&executable)
            .args(timeout)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result, json!({"version": 1, "ok": true, "value": "42"}));
        let log = std::str::from_utf8(&output.stderr).unwrap();
        assert!(log.contains("runtime stdout\n"), "{log:?}");
        assert!(log.contains("runtime stderr\n"), "{log:?}");
    }
}

#[cfg(unix)]
#[test]
fn eval_reports_exit_124_from_the_runtime_as_a_failure() {
    let fixture = Fixture::new("eval runtime exit 124");
    for (options, script, phase) in [
        (
            &["--clojure"][..],
            "#!/bin/sh\nprintf '%s' ':started' > \"$5\"\nexit 124\n",
            "execution",
        ),
        (
            &["--backend", "clojurust", "--cljrs"][..],
            "#!/bin/sh\nexit 124\n",
            "launch",
        ),
    ] {
        let executable = fixture.launcher("runtime stub", script);
        let output = fixture
            .eval_command()
            .args(["--expr", "42", "--timeout-ms", "6000"])
            .args(options)
            .arg(executable)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["version"], 1);
        assert_eq!(result["ok"], false);
        assert_eq!(result["run_error"]["phase"], phase);
        let message = result["run_error"]["message"].as_str().unwrap();
        assert!(message.contains("124"), "{message}");
        assert!(!message.contains("timed out"), "{message}");
    }
}

#[test]
#[ignore = "requires the Clojure CLI and JVM"]
fn eval_calls_a_project_function() {
    let fixture = Fixture::new("eval project function");
    let output = fixture.eval(&[
        "--expr",
        "(require 'demo.routing) (demo.routing/status {:route :home})",
    ]);
    assert!(output.status.success(), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result, json!({"version": 1, "ok": true, "value": ":found"}));
}

#[test]
#[ignore = "requires the Clojure CLI and JVM"]
fn eval_keeps_namespace_state_and_prints_the_value_separately() {
    let fixture = Fixture::new("eval forms");
    let output = fixture.eval(&[
        "--alias",
        ":test",
        "--expr",
        "(ns eval.scratch (:require [demo.routing-test])) (def label \"Привет 🐈\") ; end",
        "--expr",
        "(println (ns-name *ns*) label) (binding [*out* *err*] (println \"diagnostic\"))",
        "--expr",
        "[(str label \" 世界\") (System/getProperty \"parenox.test.flag\") ::marker]",
    ]);
    assert!(output.status.success(), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        result["value"],
        "[\"Привет 🐈 世界\" \"from-alias\" :eval.scratch/marker]"
    );
    let log = String::from_utf8(output.stderr).unwrap();
    assert!(log.contains("eval.scratch Привет 🐈\n"), "{log:?}");
    assert!(log.contains("diagnostic\n"), "{log:?}");
}

#[test]
#[ignore = "requires the Clojure CLI and JVM"]
fn eval_distinguishes_nil_from_an_exception() {
    let fixture = Fixture::new("eval exceptions");
    let nil = fixture.eval(&["--expr", "(println \"before nil\") nil"]);
    assert!(nil.status.success(), "{nil:?}");
    let result: Value = serde_json::from_slice(&nil.stdout).unwrap();
    assert_eq!(result["value"], "nil");

    let failure = fixture.eval(&[
        "--expr",
        "(println \"before exception\") (throw (ex-info \"сломалось\" {}))",
        "--expr",
        "(println \"after exception\")",
    ]);
    assert_eq!(failure.status.code(), Some(2), "{failure:?}");
    let result: Value = serde_json::from_slice(&failure.stdout).unwrap();
    assert_eq!(result["ok"], false);
    assert!(result.get("value").is_none());
    assert_eq!(result["run_error"]["phase"], "execution");
    assert_eq!(
        result["run_error"]["exception"],
        json!({"class":"clojure.lang.ExceptionInfo","message":"сломалось"})
    );
    let log = String::from_utf8(failure.stderr).unwrap();
    assert!(log.contains("before exception"), "{log:?}");
    assert!(!log.contains("after exception"), "{log:?}");

    let incomplete = fixture.eval(&["--expr", "(+ 1", "--expr", "2)"]);
    assert_eq!(incomplete.status.code(), Some(2), "{incomplete:?}");
    let result: Value = serde_json::from_slice(&incomplete.stdout).unwrap();
    assert_eq!(result["run_error"]["phase"], "execution");
}

#[test]
#[ignore = "requires the Clojure CLI and JVM"]
fn run_matches_clojure_cli_for_namespace_alias_and_script() {
    let fixture = Fixture::new("run application");
    fixture.install_app();
    let run = |command: &mut Command| {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all("Юлия\n".as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };

    for (parenox, clojure, code) in [
        (
            &[
                "--alias",
                "run-deps",
                "--main",
                "demo.app",
                "--",
                "--flag",
                "δοκιμή",
            ][..],
            &["-M:run-deps", "-m", "demo.app", "--flag", "δοκιμή"][..],
            17,
        ),
        (
            &[
                "--alias",
                "run-deps",
                "--alias",
                "run-main",
                "--timeout-ms",
                "20000",
                "--",
                "--flag",
                "δοκιμή",
            ],
            &["-M:run-deps:run-main", "--flag", "δοκιμή"],
            17,
        ),
        (
            &["script.clj", "--", "--flag", "δοκιμή"],
            &["-M", "script.clj", "--flag", "δοκιμή"],
            0,
        ),
    ] {
        let direct = run(Command::new("clojure")
            .current_dir(&fixture.0)
            .args(clojure));
        let actual = run(fixture.app().args(parenox));
        assert_eq!(direct.status.code(), Some(code), "{direct:?}");
        assert_eq!(actual.status, direct.status, "{actual:?}");
        assert_eq!(actual.stdout, direct.stdout, "{actual:?}");
        assert_eq!(actual.stderr, direct.stderr, "{actual:?}");
        let stdout = String::from_utf8(actual.stdout).unwrap();
        assert_eq!(stdout.lines().count(), 1, "{stdout:?}");
        assert!(stdout.contains("\"--flag\" \"δοκιμή\""), "{stdout:?}");
        if code == 17 {
            assert!(stdout.contains("Welcome, Юлия"), "{stdout:?}");
            assert!(
                stdout.contains(fixture.0.canonicalize().unwrap().to_str().unwrap()),
                "{stdout:?}"
            );
            assert_eq!(actual.stderr, b"app diagnostic\n");
        }
    }
}

#[cfg(unix)]
#[test]
fn run_reaps_the_child_after_timeout_and_cancellation() {
    for (name, timeout, signal, code) in [
        ("run cancellation", "8000", Some(libc::SIGTERM), 143),
        ("run timeout", "3000", None, 124),
    ] {
        let fixture = Fixture::new(name);
        let executable = fixture.launcher(
            "clojure stub",
            r#"#!/bin/sh
printf '%s' "$$" > child.pid
: > child.ready
exec /bin/sleep 10
"#,
        );
        let child = fixture
            .app()
            .args(["--alias", "run-main", "--timeout-ms", timeout, "--clojure"])
            .arg(&executable)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = match signal {
            Some(signal) => fixture.signal_after_runtime_starts(child, signal),
            None => child.wait_with_output().unwrap(),
        };
        assert_eq!(output.status.code(), Some(code), "{output:?}");
        if signal.is_none() {
            assert!(String::from_utf8_lossy(&output.stderr).contains("timed out"));
        }
        fixture.assert_child_reaped(&output);
    }
}

#[cfg(unix)]
const JVM_TEST_LAUNCHER: &[&str] = &["--alias", "test", "--clojure"];
#[cfg(unix)]
const NATIVE_TEST_LAUNCHER: &[&str] = &["--backend", "clojurust", "--src-path", "test", "--cljrs"];
#[cfg(unix)]
const TEST_NAMESPACE: &[&str] = &["demo.routing-test"];
#[cfg(unix)]
const TEST_VAR: &[&str] = &["--var", "demo.routing-test/route-status"];

#[cfg(unix)]
#[test]
fn test_times_out_and_reaps_the_jvm_in_both_formats() {
    for format in ["text", "json"] {
        assert_timed_out_test("jvm namespace", JVM_TEST_LAUNCHER, format, TEST_NAMESPACE);
    }
}

#[cfg(unix)]
#[test]
fn test_times_out_and_reaps_the_native_namespace_runner_in_both_formats() {
    for format in ["text", "json"] {
        assert_timed_out_test(
            "clojurust namespace",
            NATIVE_TEST_LAUNCHER,
            format,
            TEST_NAMESPACE,
        );
    }
}

#[cfg(unix)]
#[test]
fn test_times_out_and_reaps_the_native_var_runner_in_text_format() {
    assert_timed_out_test("clojurust var", NATIVE_TEST_LAUNCHER, "text", TEST_VAR);
}

#[cfg(unix)]
fn assert_timed_out_test(label: &str, backend: &[&str], format: &str, selectors: &[&str]) {
    let fixture = Fixture::new(&format!("test timeout {label} {format}"));
    let executable = fixture.launcher("runtime stub", TIMEOUT_STUB);
    let output = fixture
        .test_command()
        .args(backend)
        .arg(executable)
        .args(["--format", format, "--timeout-ms", "3000"])
        .args(selectors)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_test_interrupted(&output, 124, format, "Clojure test timed out after 3000 ms");
    fixture.assert_child_reaped(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("descendant started"));
    // Captured streams stay open until a surviving descendant can write its marker.
    assert!(!fixture.0.join("descendant-survived").exists(), "{label}");
}

#[cfg(unix)]
#[test]
fn test_cancels_a_supervised_test_run_on_sigterm() {
    let fixture = Fixture::new("test cancellation");
    let executable = fixture.launcher("runtime stub", CANCELLATION_STUB);
    let child = fixture
        .test_command()
        .args(JVM_TEST_LAUNCHER)
        .arg(executable)
        .args(["--format", "json", "--timeout-ms", "8000"])
        .args(TEST_NAMESPACE)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = fixture.signal_after_runtime_starts(child, libc::SIGTERM);
    assert_test_interrupted(
        &output,
        143,
        "json",
        &format!("Clojure test cancelled by signal {}", libc::SIGTERM),
    );
    fixture.assert_child_reaped(&output);
}

#[cfg(unix)]
fn assert_test_interrupted(output: &Output, code: i32, format: &str, message: &str) {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    let stderr = assert_test_logs(output, format);
    let diagnostic = format!("parenox: {message}");
    assert!(stderr.lines().any(|line| line == diagnostic), "{stderr:?}");
    if format == "json" {
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            report,
            json!({
                "version": 1, "ok": false, "tests": 0, "assertions": 0,
                "failures": 0, "errors": 0, "findings": [],
                "run_error": {"phase": "execution", "message": message}
            })
        );
    }
}

#[cfg(unix)]
fn assert_test_logs<'a>(output: &'a Output, format: &str) -> &'a str {
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert!(stderr.contains("runtime stderr\n"), "{output:?}");
    if format == "json" {
        assert!(stderr.contains("runtime stdout\n"), "{output:?}");
    } else {
        assert_eq!(output.stdout, b"runtime stdout\n", "{output:?}");
        assert!(!stderr.contains("runtime stdout\n"), "{output:?}");
    }
    stderr
}

#[cfg(unix)]
#[test]
fn test_preserves_completed_reports_and_raw_native_exit_codes_with_a_timeout() {
    let fixture = Fixture::new("test completed with timeout");
    let executable = fixture.launcher(
        "runtime stub",
        r#"#!/bin/sh
printf 'runtime stdout\n'
printf 'runtime stderr\n' >&2
case "$3" in
    test) exit "$PARENOX_TEST_STUB_EXIT" ;;
    run)
        while [ "$#" -gt 0 ] && [ "$1" != -- ]; do shift; done
        shift
        result=$1 ;;
    -M*) result=$5 ;;
    *) exit 9 ;;
esac
if [ "$PARENOX_TEST_STUB_EXIT" = 124 ]; then
    printf '%s' ':started' > "$result"
    exit 124
fi
printf '%s' '[1 1 0 0 []]' > "$result"
"#,
    );

    for (backend, format, selectors) in [
        (JVM_TEST_LAUNCHER, "json", TEST_NAMESPACE),
        (JVM_TEST_LAUNCHER, "text", TEST_NAMESPACE),
        (NATIVE_TEST_LAUNCHER, "json", TEST_NAMESPACE),
        (NATIVE_TEST_LAUNCHER, "text", TEST_VAR),
    ] {
        let output = fixture
            .test_command()
            .args(backend)
            .arg(&executable)
            .env("PARENOX_TEST_STUB_EXIT", "0")
            .args(["--format", format, "--timeout-ms", "6000"])
            .args(selectors)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_test_logs(&output, format);
        if format == "json" {
            let report: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(
                report,
                json!({
                    "version": 1, "ok": true, "tests": 1, "assertions": 1,
                    "failures": 0, "errors": 0, "findings": []
                })
            );
        }
    }

    for timeout in [&[][..], &["--timeout-ms", "6000"][..]] {
        let output = fixture
            .test_command()
            .args(NATIVE_TEST_LAUNCHER)
            .arg(&executable)
            .env("PARENOX_TEST_STUB_EXIT", "124")
            .args(timeout)
            .args(TEST_NAMESPACE)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(124), "{output:?}");
        assert_eq!(assert_test_logs(&output, "text"), "runtime stderr\n");
    }

    for backend in [JVM_TEST_LAUNCHER, NATIVE_TEST_LAUNCHER] {
        let output = fixture
            .test_command()
            .args(backend)
            .arg(&executable)
            .env("PARENOX_TEST_STUB_EXIT", "124")
            .args(["--format", "json", "--timeout-ms", "6000"])
            .args(TEST_NAMESPACE)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert_test_logs(&output, "json");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["ok"], false);
        assert_eq!(report["run_error"]["phase"], "execution");
        let message = report["run_error"]["message"].as_str().unwrap();
        assert!(message.contains("124"), "{message}");
        assert!(!message.contains("timed out"), "{message}");
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
#[ignore = "requires the Clojure CLI and JVM"]
fn distinguishes_startup_errors_from_early_jvm_exit() {
    let fixture = Fixture::new("incomplete JVM");
    fs::write(fixture.0.join("deps.edn"), "{:paths [\"src\"}").unwrap();
    let launch = fixture.run(&["demo.routing-test"], &["--format", "json"]);
    assert_eq!(launch.status.code(), Some(2), "{launch:?}");
    let report: Value = serde_json::from_slice(&launch.stdout).unwrap();
    assert_eq!(report["run_error"]["phase"], "launch");

    fs::write(fixture.0.join("deps.edn"), MANIFEST).unwrap();
    fs::write(
        fixture.0.join(TEST_PATH),
        "(ns demo.routing-test (:require [clojure.test :refer [deftest]]))
         (deftest exits (System/exit 0))",
    )
    .unwrap();
    let execution = fixture.run(&["demo.routing-test"], &["--format", "json"]);
    assert_eq!(execution.status.code(), Some(2), "{execution:?}");
    let report: Value = serde_json::from_slice(&execution.stdout).unwrap();
    assert_eq!(report["run_error"]["phase"], "execution");
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

    let formatted = run_ci(&["fmt", "--check", "--alias", "test"]);
    assert!(formatted.status.success(), "{formatted:?}");
    let value = run_ci(&[
        "eval",
        "--format",
        "json",
        "--expr",
        "(require 'demo.routing) (demo.routing/status {:route :home})",
    ]);
    assert!(value.status.success(), "{value:?}");
    let report: Value = serde_json::from_slice(&value.stdout).unwrap();
    assert_eq!(report["value"], ":found");
    fs::write(fixture.0.join("script.clj"), SCRIPT).unwrap();
    let script = run_ci(&["run", "script.clj", "--", "argument"]);
    assert!(script.status.success(), "{script:?}");
    assert_eq!(script.stdout, b"[:found [\"argument\"]]\n");
    let tests = run_ci(&["test", "--alias", "test", "--discover", "--format", "json"]);
    assert!(tests.status.success(), "{tests:?}");
    let report: Value = serde_json::from_slice(&tests.stdout).unwrap();
    assert_eq!(report["ok"], true);

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

    let focused_path = "test/demo/focused_test.clj";
    let focused = include_str!("fixtures/focused_test.clj");
    fs::write(fixture.0.join(focused_path), focused).unwrap();
    let checked = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .current_dir(&fixture.0)
        .args([
            "check",
            "--allow-incomplete",
            "--format",
            "json",
            focused_path,
        ])
        .output()
        .unwrap();
    assert_eq!(checked.status.code(), Some(0), "{checked:?}");
    let report: Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(report["complete"], false);
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

    let broken_fixture = focused.replace(
        "(println \"once fixture\")",
        "(throw (ex-info \"setup wrapper\" {} (IllegalStateException. \"setup failed\")))",
    );
    fs::write(fixture.0.join(focused_path), broken_fixture).unwrap();
    let output = fixture.run(&["--var", first], &["--format", "json"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    let error = &result["run_error"];
    assert_eq!(error["phase"], "execution");
    assert_eq!(error["path"], focused_path);
    assert_eq!(error["line"], 7);
    assert_eq!(
        error["exception"]["class"],
        "java.lang.IllegalStateException"
    );
    assert_eq!(error["exception"]["message"], "setup failed");
}

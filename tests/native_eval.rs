use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("parenox native eval {name} {}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        fs::create_dir(fixture.0.join("empty")).unwrap();
        fixture.write("deps.edn", include_str!("fixtures/jvm-project/deps.edn"));
        fixture.write(
            "src/portable/eval_helper.cljc",
            include_str!("fixtures/portable-runtime/native-eval/eval_helper.cljc"),
        );
        fixture.write("input/message.txt", "世界");
        fixture
    }

    fn write(&self, path: &str, text: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
        command
            .current_dir(self.0.join("empty"))
            .args(["eval", "--project"])
            .arg(&self.0);
        command
    }

    fn native(&self, executable: &Path) -> Command {
        let empty = self.0.join("empty");
        let mut command = self.command();
        command
            .env_clear()
            .env("PATH", &empty)
            .env("HOME", &empty)
            .args(["--backend", "clojurust", "--src-path", "src", "--cljrs"])
            .arg(executable);
        command
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

fn evaluate(command: &mut Command, format: &str, expressions: &[&str]) -> Output {
    command.args(["--format", format]);
    for expression in expressions {
        command.args(["--expr", expression]);
    }
    command.output().unwrap()
}

fn report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error}: {output:?}"))
}

fn failure(output: &Output, phase: &str) -> Value {
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let report = report(output);
    assert_eq!(report["version"], 1);
    assert_eq!(report["ok"], false);
    assert!(report.get("value").is_none(), "{report}");
    assert_eq!(report["run_error"]["phase"], phase, "{report}");
    report["run_error"].clone()
}

#[test]
#[ignore = "requires the Clojure CLI, JVM and the pinned cljrs via PARENOX_CLJRS"]
fn native_eval_matches_jvm_state_values_and_program_logs() {
    let fixture = Fixture::new("shared forms");
    let binary = native_binary();
    let expressions = [
        "(ns native.eval-demo) (def label \"Привет 🐈\") ; comment without a final newline",
        "(require '[portable.eval-helper :as helper])",
        "(println \"program log\" (helper/project-message)) (defn -main [] (println \"unexpected -main\"))",
        "[label (helper/project-message) ::marker ::helper/item]",
        "; comment-only expression",
        "#_(println \"discarded expression\")",
    ];

    let mut native = fixture.native(&binary);
    #[cfg(unix)]
    {
        let temporary = fixture.0.join(r#"temporary "quoted"\path"#);
        fs::create_dir(&temporary).unwrap();
        native.env("TMPDIR", temporary);
    }
    let jvm = evaluate(&mut fixture.command(), "json", &expressions);
    let native = evaluate(&mut native, "json", &expressions);
    for output in [jvm, native] {
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stderr, "program log 世界\n".as_bytes(), "{output:?}");
        assert_eq!(
            report(&output),
            json!({
                "version": 1,
                "ok": true,
                "value": "[\"Привет 🐈\" \"世界\" :native.eval-demo/marker :portable.eval-helper/item]"
            }),
        );
    }
}

#[test]
#[ignore = "requires the Clojure CLI, JVM and the pinned cljrs via PARENOX_CLJRS"]
fn native_eval_distinguishes_nil_text_and_an_empty_batch() {
    let fixture = Fixture::new("nil and text");
    let binary = native_binary();
    for mut command in [fixture.command(), fixture.native(&binary)] {
        let output = evaluate(
            &mut command,
            "text",
            &["(println \"before nil\") nil", "#_42", "; trailing comment"],
        );
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"nil\n");
        assert_eq!(output.stderr, b"before nil\n");
    }
    for mut command in [fixture.command(), fixture.native(&binary)] {
        let output = evaluate(&mut command, "json", &["; no forms", "#_42"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            report(&output),
            json!({"version": 1, "ok": true, "value": "nil"})
        );
        assert!(output.stderr.is_empty(), "{output:?}");
    }
    let output = evaluate(&mut fixture.native(&binary), "text", &["-42"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"-42\n");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
#[ignore = "requires the Clojure CLI, JVM and the pinned cljrs via PARENOX_CLJRS"]
fn native_eval_matches_jvm_exception_messages_and_preserves_expression_boundaries() {
    let fixture = Fixture::new("failed forms");
    let binary = native_binary();
    let expressions = [
        "(println \"before exception\") (throw (ex-info \"wrapper\" {} (ex-info \"root eval fault 🐈\" {})))",
        "(println \"after exception\")",
    ];
    for mut command in [fixture.command(), fixture.native(&binary)] {
        let output = evaluate(&mut command, "json", &expressions);
        let error = failure(&output, "execution");
        assert_eq!(error["message"], "root eval fault 🐈", "{error}");
        let log = std::str::from_utf8(&output.stderr).unwrap();
        assert!(log.contains("before exception\n"), "{log}");
        assert!(!log.contains("after exception"), "{log}");
    }

    for mut command in [fixture.command(), fixture.native(&binary)] {
        let output = evaluate(
            &mut command,
            "json",
            &["(+ 1", "2)", "(println \"after invalid read\")"],
        );
        failure(&output, "execution");
        let log = std::str::from_utf8(&output.stderr).unwrap();
        assert!(!log.contains("after invalid read"), "{log}");
    }
}

#[test]
fn native_eval_classifies_backend_and_launch_errors_without_a_jvm() {
    let fixture = Fixture::new("backend errors");
    let executable = fixture.0.join("missing-cljrs");
    let missing = evaluate(&mut fixture.native(&executable), "json", &["42"]);
    let error = failure(&missing, "launch");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("could not start"),
        "{error}"
    );

    let bad_native = evaluate(
        fixture.native(&executable).args(["--alias", "test"]),
        "json",
        &["42"],
    );
    let error = failure(&bad_native, "selection");
    assert!(
        error["message"].as_str().unwrap().contains("--alias"),
        "{error}"
    );

    let bad_jvm = evaluate(
        fixture.command().arg("--cljrs").arg(executable),
        "json",
        &["42"],
    );
    let error = failure(&bad_jvm, "selection");
    assert!(
        error["message"].as_str().unwrap().contains("--cljrs"),
        "{error}"
    );
}

#[test]
#[cfg(unix)]
fn native_eval_requires_a_report_even_if_the_external_backend_exits_successfully() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new("fake backend");
    fixture.write(
        "cljrs stub",
        concat!(
            "#!/bin/sh\n",
            "printf 'external stdout\\n'\n",
            "printf 'external stderr\\n' >&2\n",
            "exit 0\n",
        ),
    );
    let executable = fixture.0.join("cljrs stub");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let output = evaluate(&mut fixture.native(&executable), "json", &["42"]);
    let error = failure(&output, "report");
    assert!(
        error["message"].as_str().unwrap().contains("eval result"),
        "{error}"
    );
    let log = std::str::from_utf8(&output.stderr).unwrap();
    assert!(log.contains("external stdout\n"), "{log}");
    assert!(log.contains("external stderr\n"), "{log}");
}

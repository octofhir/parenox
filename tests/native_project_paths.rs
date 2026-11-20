use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const MANIFEST: &str = include_str!("fixtures/portable-project-paths/deps.edn");
const TEST_SOURCE: &str = include_str!("fixtures/portable-project-paths/fixture_test.cljc");

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "parenox project paths {name} {}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        fs::create_dir(fixture.0.join("caller")).unwrap();
        for name in [
            "base",
            "first",
            "second",
            "replacement",
            "legacy",
            "shared",
            "common-tail",
        ] {
            let directory = fixture.project().join(name).join("portable");
            fs::create_dir_all(&directory).unwrap();
            fs::write(
                directory.join("choice.cljc"),
                format!(
                    "(ns portable.choice) (def selected :{name})\n(defn -main [& args] (prn [selected (vec args)]))\n"
                ),
            )
            .unwrap();
        }
        fs::write(fixture.project().join("deps.edn"), MANIFEST).unwrap();
        fs::write(
            fixture.project().join("first/portable/fixture_test.cljc"),
            TEST_SOURCE,
        )
        .unwrap();
        fixture
    }

    fn project(&self) -> PathBuf {
        self.0.join("project")
    }

    fn command(&self, action: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
        command.current_dir(self.0.join("caller")).args([
            action,
            "--project",
            "../project",
            "--timeout-ms",
            "30000",
        ]);
        command
    }

    fn native(&self, action: &str, cljrs: &Path) -> Command {
        let empty = self.0.join("caller");
        let mut command = self.command(action);
        command
            .env_clear()
            .env("PATH", &empty)
            .env("TMPDIR", &empty)
            .args(["--backend", "clojurust", "--cljrs"])
            .arg(cljrs);
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error}: {output:?}"))
}

fn assert_prelaunch_error(output: &Output, phase: &str, expected: &str) {
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let report = report(output);
    assert_eq!(report["version"], 1);
    assert_eq!(report["ok"], false);
    assert_eq!(report["run_error"]["phase"], phase);
    let message = report["run_error"]["message"].as_str().unwrap();
    assert!(message.contains(expected), "{message}");
    assert!(!message.contains("could not start"), "{message}");
}

#[test]
fn rejects_incompatible_path_options_before_launching_a_runtime() {
    let fixture = Fixture::new("invalid options");
    let absent = fixture.0.join("missing runtime");
    for (options, expected) in [
        (&["--alias", "first"][..], "--alias"),
        (&["--deps-paths", "--clojure", "ignored"][..], "--clojure"),
    ] {
        let output = fixture
            .native("eval", &absent)
            .args(["--expr", "42", "--format", "json"])
            .args(options)
            .output()
            .unwrap();
        assert_prelaunch_error(&output, "selection", expected);
    }

    let output = fixture
        .command("eval")
        .args(["--deps-paths", "--expr", "42", "--format", "json"])
        .output()
        .unwrap();
    assert_prelaunch_error(&output, "selection", "--deps-paths");

    let output = fixture
        .native("eval", &absent)
        .args([
            "--deps-paths",
            "--src-path",
            "base",
            "--expr",
            "42",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert!(
        stderr.contains("--deps-paths") && stderr.contains("--src-path"),
        "{stderr}"
    );
    assert!(!stderr.contains("could not start"), "{stderr}");
}

#[test]
fn reports_missing_native_project_manifest_in_json() {
    let fixture = Fixture::new("missing manifest");
    fs::remove_file(fixture.project().join("deps.edn")).unwrap();
    let output = fixture
        .native("eval", &fixture.0.join("missing runtime"))
        .args(["--deps-paths", "--expr", "42", "--format", "json"])
        .output()
        .unwrap();
    assert_prelaunch_error(&output, "project", "deps.edn");
}

#[cfg(unix)]
#[test]
fn native_project_paths_pass_only_ordered_roots_to_the_launcher() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new("launcher paths");
    let launcher = fixture.0.join("runtime stub");
    fs::write(&launcher, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
    let output = fixture
        .native("run", &launcher)
        .args([
            "--deps-paths",
            "--alias",
            ":inert",
            "--main",
            "portable.choice",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let lines: Vec<_> = std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .collect();
    let [
        "--no-config",
        "--no-native",
        "run",
        "--src-path",
        first,
        "--src-path",
        base,
        "--src-path",
        shared,
        "--src-path",
        common_tail,
        script,
    ] = lines.as_slice()
    else {
        panic!("unexpected launcher arguments: {lines:?}");
    };
    for (argument, relative) in [
        (first, "first"),
        (base, "base"),
        (shared, "shared"),
        (common_tail, "common-tail"),
        (script, "first/portable/choice.cljc"),
    ] {
        assert_eq!(
            Path::new(argument).canonicalize().unwrap(),
            fixture.project().join(relative).canonicalize().unwrap()
        );
    }
}

fn aliases(command: &mut Command, aliases: &[&str]) {
    for alias in aliases {
        command.args(["--alias", alias]);
    }
}

fn compare(
    fixture: &Fixture,
    cljrs: &Path,
    action: &str,
    selected: &[&str],
    args: &[&str],
) -> Output {
    let mut jvm = fixture.command(action);
    aliases(&mut jvm, selected);
    let jvm = jvm.args(args).output().unwrap();
    assert!(jvm.status.success(), "{action} on JVM: {jvm:?}");

    let mut native = fixture.native(action, cljrs);
    native.arg("--deps-paths");
    aliases(&mut native, selected);
    let native = native.args(args).output().unwrap();
    assert_eq!(native.status, jvm.status, "{action} on native: {native:?}");
    assert_eq!(
        native.stdout, jvm.stdout,
        "{action} stdout for {selected:?}"
    );
    assert_eq!(
        native.stderr, jvm.stderr,
        "{action} stderr for {selected:?}"
    );
    native
}

#[test]
#[ignore = "requires Clojure CLI, JVM and PARENOX_CLJRS at 793ad97e"]
fn manifest_roots_select_the_same_portable_namespaces_on_jvm_and_native() {
    let cljrs = PathBuf::from(
        std::env::var_os("PARENOX_CLJRS").expect("set PARENOX_CLJRS to pinned cljrs"),
    )
    .canonicalize()
    .unwrap();
    let fixture = Fixture::new("JVM and native");
    for (selected, expected) in [
        (&[][..], ":base"),
        (&["first", "second"][..], ":first"),
        (&["second", "first"][..], ":second"),
        (&["replace"][..], ":replacement"),
        (&["first", "replace"][..], ":first"),
        (&["replace", "legacy"][..], ":legacy"),
    ] {
        let output = compare(
            &fixture,
            &cljrs,
            "run",
            selected,
            &["--main", "portable.choice", "--", "with space", "λ"],
        );
        assert_eq!(
            output.stdout,
            format!("[{expected} [\"with space\" \"λ\"]]\n").as_bytes()
        );
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    let output = compare(
        &fixture,
        &cljrs,
        "eval",
        &["second", "first"],
        &[
            "--format",
            "json",
            "--expr",
            "(require '[portable.choice :as choice]) (println \"eval λ\") choice/selected",
        ],
    );
    assert_eq!(
        report(&output),
        json!({"version": 1, "ok": true, "value": ":second"})
    );
    assert_eq!(output.stderr, "eval λ\n".as_bytes());

    let output = compare(
        &fixture,
        &cljrs,
        "test",
        &["first"],
        &["--format", "json", "--discover"],
    );
    assert_eq!(
        report(&output),
        json!({
            "version": 1, "ok": true, "tests": 1, "assertions": 1,
            "failures": 0, "errors": 0, "findings": []
        })
    );
    assert_eq!(
        output.stderr,
        b"\nTesting portable.fixture-test\nselected :first\n\nRan 1 tests containing 1 assertions.\n0 failures, 0 errors.\n"
    );
}

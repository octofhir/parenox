use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const MEDLEY_REVISION: &str = "54f45abc16ecd30121d05ef52014af8c65d4635f";
const UPSTREAM_FILES: &[&str] = &["src/medley/core.cljc", "test/medley/core_test.cljc"];
const PORTABLE_FILES: &[(&str, &[u8])] = &[
    (
        "deps.edn",
        include_bytes!("fixtures/portable-medley/deps.edn"),
    ),
    (
        "src/portable/medley_demo.cljc",
        include_bytes!("fixtures/portable-medley/src/portable/medley_demo.cljc"),
    ),
    (
        "test/portable/medley_demo_test.cljc",
        include_bytes!("fixtures/portable-medley/test/portable/medley_demo_test.cljc"),
    ),
];

struct Fixture(PathBuf);

impl Fixture {
    fn new(checkout: &Path) -> Self {
        let root =
            std::env::temp_dir().join(format!("parenox portable Medley {}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        fs::create_dir(fixture.0.join("empty")).unwrap();
        for (path, contents) in PORTABLE_FILES {
            fixture.write(path, contents);
        }
        for path in UPSTREAM_FILES {
            let object = format!("{MEDLEY_REVISION}:{path}");
            let output = Command::new("git")
                .arg("-C")
                .arg(checkout)
                .args(["--no-replace-objects", "show", "--no-textconv", &object])
                .env("GIT_NO_LAZY_FETCH", "1")
                .output()
                .expect("run git show for Medley");
            assert!(
                output.status.success(),
                "git show {object} in {}: {}",
                checkout.display(),
                String::from_utf8_lossy(&output.stderr)
            );
            fixture.write(path, &output.stdout);
        }
        fixture
    }

    fn write(&self, path: &str, contents: &[u8]) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn command(&self, action: &str, native: Option<&Path>, source_roots: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
        let empty = self.0.join("empty");
        command
            .current_dir(&empty)
            .args([action, "--timeout-ms", "20000", "--project"])
            .arg(&self.0);
        if let Some(cljrs) = native {
            command
                .env_clear()
                .env("PATH", &empty)
                .env("TMPDIR", &empty)
                .args(["--backend", "clojurust", "--cljrs"])
                .arg(cljrs);
            for root in source_roots {
                command.args(["--src-path", root]);
            }
        }
        command
    }

    fn test(&self, namespace: &str, native: Option<&Path>) -> Output {
        self.command("test", native, &["src", "test"])
            .args(["--format", "json", namespace])
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn required_path(variable: &str) -> PathBuf {
    let path =
        std::env::var_os(variable).unwrap_or_else(|| panic!("set {variable} to a local path"));
    PathBuf::from(path)
        .canonicalize()
        .unwrap_or_else(|error| panic!("{variable}: {error}"))
}

fn assert_same_success(case: &str, jvm: &Output, native: &Output) {
    assert!(jvm.status.success(), "{case} on JVM: {jvm:?}");
    assert_eq!(native.status, jvm.status, "{case} on native: {native:?}");
    assert_eq!(native.stdout, jvm.stdout, "{case} stdout");
    assert_eq!(native.stderr, jvm.stderr, "{case} stderr");
}

fn report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error}: {output:?}"))
}

#[test]
#[ignore = "requires Clojure CLI, JVM, PARENOX_CLJRS at 793ad97e and PARENOX_MEDLEY at 54f45abc"]
fn pinned_medley_runs_a_portable_consumer_and_reports_the_upstream_native_load_gap() {
    let cljrs = required_path("PARENOX_CLJRS");
    let fixture = Fixture::new(&required_path("PARENOX_MEDLEY"));

    let entry = |native| {
        fixture
            .command("run", native, &["src"])
            .args(["--main", "portable.medley-demo", "--", "a b", "λ"])
            .output()
            .unwrap()
    };
    let jvm_entry = entry(None);
    let native_entry = entry(Some(&cljrs));
    assert_same_success("Medley entry point", &jvm_entry, &native_entry);
    assert_eq!(
        jvm_entry.stdout,
        "medley scenario λ\n[[\"a b\" \"λ\"] [5 [1 2] {:keep 9} [false 7 false]]]\n".as_bytes()
    );
    assert!(jvm_entry.stderr.is_empty());

    let jvm_portable = fixture.test("portable.medley-demo-test", None);
    let native_portable = fixture.test("portable.medley-demo-test", Some(&cljrs));
    assert_same_success("portable Medley test", &jvm_portable, &native_portable);
    assert_eq!(
        report(&jvm_portable),
        json!({
            "version": 1, "ok": true, "tests": 1, "assertions": 4,
            "failures": 0, "errors": 0, "findings": []
        })
    );

    let jvm_upstream = fixture.test("medley.core-test", None);
    assert!(jvm_upstream.status.success(), "{jvm_upstream:?}");
    assert_eq!(
        report(&jvm_upstream),
        json!({
            "version": 1, "ok": true, "tests": 53, "assertions": 280,
            "failures": 0, "errors": 0, "findings": []
        })
    );

    // Upstream imports the test macros through :clj and :cljs branches.
    let native_upstream = fixture.test("medley.core-test", Some(&cljrs));
    let message = "runtime error: in medley.core-test: unbound symbol: deftest";
    assert_eq!(
        native_upstream.status.code(),
        Some(2),
        "{native_upstream:?}"
    );
    assert_eq!(
        report(&native_upstream),
        json!({
            "version": 1, "ok": false, "tests": 0, "assertions": 0,
            "failures": 0, "errors": 0, "findings": [],
            "run_error": {"phase": "load", "message": message}
        })
    );
    assert_eq!(
        native_upstream.stderr,
        format!("parenox: {message}\n").as_bytes()
    );
}

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("parenox native main {name} {}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        fs::create_dir(fixture.0.join("empty")).unwrap();
        fixture
    }

    fn write(&self, path: &str, text: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn without_java(&self, command: &mut Command) {
        let empty = self.0.join("empty");
        command.env_clear().env("PATH", &empty).env("HOME", &empty);
    }

    fn main(&self, binary: &Path, roots: &[&str], namespace: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
        self.without_java(&mut command);
        command
            .current_dir(self.0.join("empty"))
            .args([
                "run",
                "--backend",
                "clojurust",
                "--timeout-ms",
                "20000",
                "--project",
            ])
            .arg(&self.0)
            .arg("--cljrs")
            .arg(binary);
        for root in roots {
            command.args(["--src-path", root]);
        }
        command.args(["--main", namespace]);
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

#[test]
#[cfg(unix)]
#[ignore = "requires the pinned cljrs via PARENOX_CLJRS"]
fn native_main_awaits_async_work_and_forwards_args_streams_and_exit_status() {
    use std::io::Write;
    use std::process::Stdio;

    let fixture = Fixture::new("async");
    let binary = native_binary();
    const SCRIPT: &str = "src/portable/launcher.cljc";
    fixture.write(
        SCRIPT,
        include_str!("fixtures/portable-runtime/native-main/launcher.cljc"),
    );
    fixture.write("input/message.txt", "project data");

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
            .write_all("вход".as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };

    for (flag, status) in [("--flag", 0), ("--fail", 1)] {
        let args = ["--", flag, "δοκιμή"];
        let mut reference = Command::new(&binary);
        fixture.without_java(&mut reference);
        reference
            .current_dir(&fixture.0)
            .args([
                "--no-config",
                "--no-native",
                "run",
                "--src-path",
                "src",
                SCRIPT,
            ])
            .args(args);
        let direct = run(&mut reference);
        let wrapped = run(fixture
            .main(&binary, &["src"], "portable.launcher")
            .args(args));
        assert_eq!(direct.status.code(), Some(status), "{direct:?}");
        assert_eq!(wrapped.status, direct.status, "{wrapped:?}");
        assert_eq!(wrapped.stdout, direct.stdout, "{wrapped:?}");
        assert_eq!(wrapped.stderr, direct.stderr, "{wrapped:?}");
        assert_eq!(
            wrapped.stdout,
            format!(
                "async main completed\n[[\"project data\" \"вход\"] [\"{flag}\" \"δοκιμή\"]]\n"
            )
            .as_bytes()
        );
        let stderr = std::str::from_utf8(&wrapped.stderr).unwrap();
        if status == 0 {
            assert!(stderr.is_empty(), "{stderr}");
        } else {
            assert!(stderr.contains("requested native main failure"), "{stderr}");
        }
    }
}

#[test]
#[ignore = "requires the pinned cljrs via PARENOX_CLJRS"]
fn native_main_prefers_the_first_root_then_the_native_extension() {
    let fixture = Fixture::new("source order");
    let binary = native_binary();
    fixture.write(
        "first/portable/main.cljc",
        "(ns portable.main)\n#?(:cljrs (defn -main [] (prn :first-shared)))\n",
    );
    fixture.write(
        "second/portable/main.cljrs",
        "(ns portable.main)\n(defn -main [] (prn :second-native))\n",
    );
    let verify = |roots: &[&str], expected: &[u8]| {
        let output = fixture
            .main(&binary, roots, "portable.main")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, expected, "{roots:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    };
    verify(&["first", "second"], b":first-shared\n");
    verify(&["second", "first"], b":second-native\n");

    fixture.write(
        "first/portable/main.cljrs",
        "(ns portable.main)\n(defn -main [] (prn :first-native))\n",
    );
    verify(&["first", "second"], b":first-native\n");
}

#[test]
fn native_main_rejects_invalid_namespace_sources_before_launch() {
    let fixture = Fixture::new("selection errors");
    let binary = fixture.0.join("missing-cljrs");
    for (path, source) in [
        (
            "src/portable/no_main.cljc",
            "(ns portable.no-main)\n#?(:clj (defn -main [] :jvm-only))\n#_(defn -main [] :discarded)\n",
        ),
        (
            "fallback/portable/no_main.cljrs",
            "(ns portable.no-main)\n(defn -main [] :shadowed)\n",
        ),
        (
            "src/portable/wrong_scope.cljrs",
            "(ns portable.different)\n(defn -main [] :wrong-namespace)\n",
        ),
        (
            "src/portable/switch.cljc",
            "(ns portable.switch)\n(defn -main [] :main)\n(in-ns 'portable.different)\n",
        ),
        (
            "src/portable/jvm_only.clj",
            "(ns portable.jvm-only)\n(defn -main [] :wrong-extension)\n",
        ),
    ] {
        fixture.write(path, source);
    }

    let cases: &[(&[&str], &str, &str)] = &[
        (
            &[],
            "portable.no-main",
            "--main with clojurust requires at least one --src-path",
        ),
        (&["src"], "portable/bad", "Expected a Clojure namespace"),
        (
            &["src"],
            "portable.missing",
            "Could not find namespace portable.missing",
        ),
        (
            &["src"],
            "portable.jvm-only",
            "Could not find namespace portable.jvm-only",
        ),
        (
            &["src", "fallback"],
            "portable.no-main",
            "Expected a direct -main declaration in namespace portable.no-main",
        ),
        (
            &["src"],
            "portable.wrong-scope",
            "Expected namespace portable.wrong-scope, got a different namespace declaration",
        ),
        (&["src"], "portable.switch", "got an in-ns switch"),
    ];
    for &(roots, namespace, expected) in cases {
        let output = fixture.main(&binary, roots, namespace).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{namespace}: {output:?}");
        assert!(output.stdout.is_empty(), "{namespace}: {output:?}");
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        assert!(stderr.contains(expected), "{namespace}: {stderr}");
        assert!(!stderr.contains("could not start"), "{namespace}: {stderr}");
    }
}

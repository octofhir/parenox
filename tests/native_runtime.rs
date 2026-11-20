use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn parenox() -> Command {
    Command::new(env!("CARGO_BIN_EXE_parenox"))
}

const FIXTURES: &[(&str, &str)] = &[
    ("deps.edn", include_str!("../deps.edn")),
    (
        "src/portable/model.cljc",
        include_str!("fixtures/portable-runtime/src/portable/model.cljc"),
    ),
    (
        "summary.cljc",
        include_str!("fixtures/portable-runtime/summary.cljc"),
    ),
    (
        "branches.cljc",
        include_str!("fixtures/portable-runtime/branches.cljc"),
    ),
    (
        "failure.cljc",
        include_str!("fixtures/portable-runtime/failure.cljc"),
    ),
    (
        "arguments.cljc",
        include_str!("fixtures/portable-runtime/arguments.cljc"),
    ),
    (
        "namespace.cljc",
        include_str!("fixtures/portable-runtime/namespace.cljc"),
    ),
    (
        "src/portable/loading.cljc",
        include_str!("fixtures/portable-runtime/src/portable/loading.cljc"),
    ),
    (
        "macros.cljc",
        include_str!("fixtures/portable-runtime/macros.cljc"),
    ),
    (
        "src/portable/macros.cljc",
        include_str!("fixtures/portable-runtime/src/portable/macros.cljc"),
    ),
    (
        "bindings.cljc",
        include_str!("fixtures/portable-runtime/bindings.cljc"),
    ),
    (
        "src/portable/dynamic.cljc",
        include_str!("fixtures/portable-runtime/src/portable/dynamic.cljc"),
    ),
    (
        "exceptions.cljc",
        include_str!("fixtures/portable-runtime/exceptions.cljc"),
    ),
    (
        "streams.cljc",
        include_str!("fixtures/portable-runtime/streams.cljc"),
    ),
];

const ASYNC_FIXTURES: &[(&str, &str)] = &[
    (
        "deps.edn",
        include_str!("fixtures/portable-runtime/core-async/deps.edn"),
    ),
    (
        "channel.cljc",
        include_str!("fixtures/portable-runtime/core-async/channel.cljc"),
    ),
    (
        "timeout.cljc",
        include_str!("fixtures/portable-runtime/core-async/timeout.cljc"),
    ),
    (
        "timeout-port.cljc",
        include_str!("fixtures/portable-runtime/core-async/timeout-port.cljc"),
    ),
];

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let root =
            std::env::temp_dir().join(format!("parenox native {name} {}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        for (path, text) in files {
            let path = fixture.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        fs::create_dir(fixture.0.join("empty")).unwrap();
        fixture
    }

    fn run(&self, timeout_ms: &str) -> Command {
        let mut command = parenox();
        command
            .current_dir(&self.0)
            .args(["run", "--timeout-ms", timeout_ms]);
        command
    }

    fn jvm(&self, script: &str, args: &[&str]) -> Output {
        self.run("20000").arg(script).args(args).output().unwrap()
    }

    fn native(
        &self,
        binary: &Path,
        source_path: Option<&str>,
        script: &str,
        args: &[&str],
    ) -> Output {
        let empty = self.0.join("empty");
        let mut command = self.run("20000");
        command
            .env_clear()
            .env("PATH", &empty)
            .env("HOME", &empty)
            .args(["--backend", "clojurust", "--cljrs"])
            .arg(binary);
        if let Some(path) = source_path {
            command.args(["--src-path", path]);
        }
        command.arg(script).args(args).output().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn native_run_reports_an_unsupported_source_before_launch() {
    let output = parenox()
        .args([
            "run",
            "--backend",
            "clojurust",
            "--cljrs",
            "missing-cljrs",
            "script.clj",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("expects a .cljc or .cljrs script"));
}

#[test]
#[ignore = "requires the Clojure CLI, JVM and the pinned cljrs via PARENOX_CLJRS"]
fn portable_scripts_agree_and_select_the_runtime_reader_branch() {
    let binary = native_binary();
    let fixture = Fixture::new("scripts", FIXTURES);
    let jvm = |script, args: &[&str]| fixture.jvm(script, args);
    let native = |script, args: &[&str]| fixture.native(&binary, Some("src"), script, args);

    for (script, expected) in [
        (
            "summary.cljc",
            "portable calculation\n[15 [3 5 7] [1 2 3 4 5] [1 2 3 4 5 2] true :shared]\n",
        ),
        (
            "namespace.cljc",
            "loaded portable.loading\nnamespace ready\n[[:loaded :alias] [:loaded :refer] [:loaded :require]]\n",
        ),
        (
            "macros.cljc",
            "cross-namespace macro\n[[:definition 1] 1 [:caller 99]]\n",
        ),
        (
            "bindings.cljc",
            "dynamic unwind\n[:root :outer :inner :outer :outer :root]\n",
        ),
        (
            "exceptions.cljc",
            "caught original\nran finally\n[\"portable wrapper\" {:seen {:code :original}}]\n",
        ),
    ] {
        for output in [jvm(script, &[]), native(script, &[])] {
            assert!(output.status.success(), "{script}: {output:?}");
            assert!(output.stderr.is_empty(), "{script}: {output:?}");
            assert_eq!(output.stdout, expected.as_bytes(), "{script}");
        }
    }

    let jvm_branch = jvm("branches.cljc", &[]);
    assert!(jvm_branch.status.success(), "{jvm_branch:?}");
    assert_eq!(jvm_branch.stdout, b":jvm\n");

    let args = &["--", "--flag", "δοκιμή"];
    for output in [jvm("arguments.cljc", args), native("arguments.cljc", args)] {
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, "[\"--flag\" \"δοκιμή\"]\n".as_bytes());
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    let jvm_streams = jvm("streams.cljc", &[]);
    let native_streams = native("streams.cljc", &[]);
    assert!(jvm_streams.status.success(), "{jvm_streams:?}");
    assert!(native_streams.status.success(), "{native_streams:?}");
    assert_eq!(jvm_streams.stdout, b"standard output\n:streams\n");
    assert_eq!(jvm_streams.stderr, b"redirected output\n");
    assert_eq!(
        native_streams.stdout,
        b"standard output\nredirected output\n:streams\n"
    );
    assert!(native_streams.stderr.is_empty(), "{native_streams:?}");

    for failure in [jvm("failure.cljc", &[]), native("failure.cljc", &[])] {
        assert_eq!(failure.status.code(), Some(1), "{failure:?}");
        assert_eq!(failure.stdout, b"before exception\n");
        assert!(String::from_utf8_lossy(&failure.stderr).contains("portable failure"));
    }

    fs::remove_file(fixture.0.join("deps.edn")).unwrap();
    fs::rename(
        fixture.0.join("branches.cljc"),
        fixture.0.join("branches.cljrs"),
    )
    .unwrap();
    let native_branch = native("branches.cljrs", &[]);
    assert!(native_branch.status.success(), "{native_branch:?}");
    assert_eq!(native_branch.stdout, b":native\n");
}

#[test]
#[ignore = "requires Clojure CLI, JVM with core.async and the pinned cljrs via PARENOX_CLJRS"]
fn core_async_channel_closes_on_both_runtimes_and_timeout_ports_differ() {
    let fixture = Fixture::new("core async", ASYNC_FIXTURES);
    let binary = native_binary();
    // Allow time to compile core.async when other JVMs share the test host.
    let jvm = |script| fixture.run("60000").arg(script).output().unwrap();
    for (case, output) in [
        ("JVM channel", jvm("channel.cljc")),
        (
            "native channel",
            fixture.native(&binary, None, "channel.cljc", &[]),
        ),
        ("JVM timeout", jvm("timeout.cljc")),
    ] {
        assert_eq!(output.status.code(), Some(0), "{case}: {output:?}");
        assert_eq!(output.stdout, b"[{:answer 42} nil]\n", "{case}");
        assert!(output.stderr.is_empty(), "{case}: {output:?}");
    }

    let native = fixture.native(&binary, None, "timeout.cljc", &[]);
    assert_eq!(native.status.code(), Some(1), "{native:?}");
    assert!(native.stdout.is_empty(), "{native:?}");
    let stderr = String::from_utf8_lossy(&native.stderr);
    assert!(stderr.contains("wrong type: expected channel,"), "{stderr}");
    assert!(stderr.contains("got future"), "{stderr}");

    for (case, output) in [
        ("JVM timeout port", jvm("timeout-port.cljc")),
        (
            "native timeout port",
            fixture.native(&binary, None, "timeout-port.cljc", &[]),
        ),
    ] {
        assert_eq!(output.status.code(), Some(0), "{case}: {output:?}");
        assert_eq!(
            output.stdout, b"[:data-wins :data]\n[:timer-wins nil :after-timeout]\n",
            "{case}"
        );
        assert!(output.stderr.is_empty(), "{case}: {output:?}");
    }
}

fn native_binary() -> PathBuf {
    let binary = std::env::var_os("PARENOX_CLJRS").expect("set PARENOX_CLJRS to a pinned cljrs");
    Path::new(&binary).canonicalize().unwrap()
}

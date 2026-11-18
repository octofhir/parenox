use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

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

    fn run(&self, namespace: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_parenox"))
            .arg("test")
            .arg("--project")
            .arg(&self.0)
            .args(["--alias", ":test"])
            .args(args)
            .arg(namespace)
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
        "demo.routing-test",
        &["--clojure", missing.to_str().unwrap()],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not start"));
}

#[test]
#[ignore = "requires the Clojure CLI and JVM"]
fn project_jvm_pass_failure_and_load_error() {
    let fixture = Fixture::new("project");
    let output = fixture.run("demo.routing-test", &[]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("2 tests containing 3 assertions"));

    let edited = SOURCE.replace(":found :missing", ":found :found");
    fs::write(fixture.0.join(SOURCE_PATH), edited).unwrap();
    let output = fixture.run("demo.routing-test", &[]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!(
            "{TEST_PATH}:7: fail in demo.routing-test/route-status"
        )),
        "{stdout}"
    );

    let output = fixture.run("demo.does-not-exist", &[]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("Clojure test run did not complete"));
}

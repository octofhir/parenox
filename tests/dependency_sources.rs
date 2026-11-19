use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde_json::{Value, json};

const DEPENDENCY: &str = r#"(ns wire (:require [clojure.spec.alpha :as s]))
(s/def ::value string?)
(s/def ::range (s/keys :req-un [::value]))
(s/fdef describe :args (s/cat :data ::range) :ret string?)
(defn describe [data]  (str (:value data)))"#;

const CONSUMER: &str = r#"(ns app (:require [wire :as wire] [clojure.spec.alpha :as s]))
(s/def ::range ::wire/range)
(defn message {:t/sig '[:-> string?]} [] (wire/describe {:value "entry"}))"#;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(test: &str) -> Self {
        let root = std::env::temp_dir().join(format!("parenox-dep-{test}-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let fixture = Self {
            root: root.canonicalize().unwrap(),
        };
        for dir in ["project/src", "project/test", "vendor"] {
            fs::create_dir_all(fixture.root.join(dir)).unwrap();
        }
        for (path, content) in [
            (
                "project/parenox.edn",
                r#"{:dependency-sources ["../vendor/wire.clj"]}"#,
            ),
            (
                "project/deps.edn",
                r#"{:paths ["src"] :aliases {:test {:extra-paths ["test"]}}}"#,
            ),
            ("project/src/base.clj", "(ns  base)"),
            ("project/test/app.cljc", CONSUMER),
            ("vendor/wire.clj", DEPENDENCY),
        ] {
            fs::write(fixture.root.join(path), content).unwrap();
        }
        let classpath = std::env::join_paths(["src", "test"]).unwrap();
        fs::write(
            fixture.project().join("classpath.txt"),
            classpath.to_str().unwrap(),
        )
        .unwrap();
        fixture
    }

    fn project(&self) -> PathBuf {
        self.root.join("project")
    }

    fn dependency(&self) -> PathBuf {
        self.root.join("vendor/wire.clj")
    }

    fn command(&self, subcommand: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
        command.current_dir(self.project()).arg(subcommand);
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn file<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| Path::new(file["path"].as_str().unwrap()).ends_with(name))
        .unwrap()
}

#[test]
fn dependency_sources_add_to_deps_and_classpath_but_not_bulk_formatting() {
    let fixture = Fixture::new("cli");
    for inputs in [
        &["--deps", "deps.edn", "--alias", "test"][..],
        &["--classpath", "classpath.txt"][..],
    ] {
        let output = fixture
            .command("check")
            .args(inputs)
            .args(["--config", "parenox.edn", "--json"])
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(output.status.success(), "{report}");
        assert_eq!(report["complete"], true);
        assert_eq!(report["files"].as_array().unwrap().len(), 3);
        assert_eq!(file(&report, "wire.clj")["diagnostics"], json!([]));
        assert_eq!(file(&report, "app.cljc")["diagnostics"], json!([]));
    }

    let output = fixture
        .command("fmt")
        .args(["--write", "--alias", "test"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        fs::read_to_string(fixture.project().join("src/base.clj")).unwrap(),
        "(ns base)\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.dependency()).unwrap(),
        DEPENDENCY
    );
}

#[test]
fn refresh_rechecks_consumers_when_a_dependency_source_spec_changes_or_disappears() {
    struct Server(Child);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let fixture = Fixture::new("serve");
    let mut server = Server(
        fixture
            .command("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut input = server.0.stdin.take().unwrap();
    let mut output = BufReader::new(server.0.stdout.take().unwrap());
    let mut request = |body: Value| {
        writeln!(input, "{body}").unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let mut response: Value = serde_json::from_str(&line).unwrap();
        assert!(response["result"].is_object(), "{response}");
        response["result"].take()
    };
    let opened = request(json!({
        "op": "open_project", "root": fixture.project(), "aliases": ["test"]
    }));
    assert_eq!(opened["ok"], true, "{opened}");
    assert_eq!(opened["complete"], true, "{opened}");

    let dependency = fixture.dependency();
    fs::write(
        &dependency,
        DEPENDENCY.replace("::value string?", "::value number?"),
    )
    .unwrap();
    let changed = request(json!({"op": "refresh_project"}));
    let consumer = file(&changed, "app.cljc");
    assert_eq!(consumer["stats"]["checked_functions"], 1, "{changed}");
    assert_eq!(consumer["stats"]["reused_parses"], 1, "{changed}");
    let [diagnostic] = consumer["diagnostics"].as_array().unwrap().as_slice() else {
        panic!("{consumer}");
    };
    assert_eq!(diagnostic["code"], "PNX2322", "{diagnostic}");
    assert_eq!(diagnostic["contract"]["name"], "wire/describe");
    assert_eq!(diagnostic["contract"]["kind"], "s_fdef");
    let origin = Path::new(diagnostic["contract"]["path"].as_str().unwrap());
    assert_eq!(origin.canonicalize().unwrap(), dependency);

    fs::write(
        &dependency,
        DEPENDENCY.replace("(s/def ::range (s/keys :req-un [::value]))", ""),
    )
    .unwrap();
    let removed = request(json!({"op": "refresh_project"}));
    let consumer = file(&removed, "app.cljc");
    assert_eq!(consumer["stats"]["checked_functions"], 1, "{removed}");
    assert_eq!(consumer["complete"], false, "{consumer}");
    let diagnostics = consumer["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic["message"] == "No declaration for spec :wire/range"),
        "{consumer}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["code"] != "PNX2322"),
        "{consumer}"
    );
}

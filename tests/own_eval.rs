use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::{Value, json};

const STATE: &[&str] = &[
    "(def seed 1) (ns portable.engine) (def seed 40)",
    "(let [value (+ seed 1) value (inc value)]
       (println \"own engine λ\")
       [value user/seed (if [] :truthy :bad) (if 0 :truthy :bad)
        (if nil :bad :nil) (if false :bad :false)])",
    "; comment without a final newline",
    "#_99",
];
const ORDER: &[&str] = &["[(do (println \"vector first\") :ok)
      {(do (println \"key one\") :one) (do (println \"value one\") 1)
       (do (println \"key two\") :two) (do (println \"value two\") 2)}
      (clojure.core/+ (do (println \"arg one\") 20) (do (println \"arg two\") 22))]"];
const ORDER_LOG: &str = "vector first\nkey one\nvalue one\nkey two\nvalue two\narg one\narg two\n";
const DUPLICATE_MAP: &str = r#"{:a (do (println "first value") 1)
    (keyword "a") (do (println "duplicate value") 2)
    :later (do (println "later value") 3)}"#;
const DUPLICATE_LOG: &str = "first value\nduplicate value\nlater value\n";
const DATA: &[&str] = &[r#"[(+) (+ 41) (+ 1 2 3) (inc -1)
        -9223372036854775808 0x2a 052 2r101010
        (if false :unreachable) (if true 1 (throw (Exception. "not run")))
        (if {} :truthy :bad) (if "" :truthy :bad) (do)
        (let* [a 5 b (inc a)] b)
        (str nil :keyword false)
        'qualified/symbol '(:key [1 nil])]"#];
const DATA_VALUE: &str = r#"[0 41 6 0 -9223372036854775808 42 42 42 nil 1 :truthy :truthy nil 6 ":keywordfalse" qualified/symbol (:key [1 nil])]"#;

const READER_NAMESPACE: &[&str] = &[
    "(do (ns portable.reader) (prn ::before)) ::after",
    "[::after '::quoted]",
];
const KEYWORD_IDENTITY: &[&str] = &[
    "{(keyword nil \"a/b\") :without-namespace, :a/b :with-namespace}",
    "{(keyword \"a/b\" \"c\") :namespace-slash, (keyword \"a\" \"b/c\") :name-slash}",
];

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("parenox own eval {name} {}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        fs::create_dir(fixture.0.join("empty")).unwrap();
        fs::write(fixture.0.join("deps.edn"), include_str!("../deps.edn")).unwrap();
        fixture
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
        command
            .current_dir(self.0.join("empty"))
            .args(["eval", "--project"])
            .arg(&self.0);
        command
    }

    fn own(&self, format: &str, inputs: &[&str]) -> Output {
        let empty = self.0.join("empty");
        let mut command = self.command();
        command
            .env_clear()
            .env("PATH", &empty)
            .args(["--backend", "parenox"]);
        evaluate(&mut command, format, inputs)
    }

    fn jvm(&self, format: &str, inputs: &[&str]) -> Output {
        evaluate(&mut self.command(), format, inputs)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn evaluate(command: &mut Command, format: &str, inputs: &[&str]) -> Output {
    command.args(["--format", format]);
    for source in inputs {
        command.args(["--expr", source]);
    }
    command.output().unwrap()
}

fn report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error}: {output:?}"))
}

fn success(output: &Output, value: &str, log: &str) {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        report(output),
        json!({"version": 1, "ok": true, "value": value})
    );
    assert_eq!(output.stderr, log.as_bytes(), "{output:?}");
}

fn execution_error(output: &Output) -> Value {
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let result = report(output);
    assert_eq!(result["version"], 1);
    assert_eq!(result["ok"], false);
    assert!(result.get("value").is_none(), "{result}");
    assert_eq!(result["run_error"]["phase"], "execution", "{result}");
    result["run_error"].clone()
}

#[test]
fn own_eval_keeps_namespaces_sequential_bindings_and_truthiness_without_java() {
    let fixture = Fixture::new("state");
    success(
        &fixture.own("json", STATE),
        "[42 1 :truthy :truthy :nil :false]",
        "own engine λ\n",
    );
    success(
        &fixture.own("json", &["(ns portable.vars) (def result 42)"]),
        "#'portable.vars/result",
        "",
    );
    success(&fixture.own("json", &["(ns portable.vars)"]), "nil", "");
    success(
        &fixture.own("json", READER_NAMESPACE),
        "[:portable.reader/after :portable.reader/quoted]",
        ":user/before\n",
    );
}

#[test]
fn own_eval_preserves_evaluation_order_and_renders_clojure_values() {
    let fixture = Fixture::new("data");
    success(
        &fixture.own("json", ORDER),
        "[:ok {:one 1, :two 2} 42]",
        ORDER_LOG,
    );
    success(&fixture.own("json", DATA), DATA_VALUE, "");
    success(
        &fixture.own("json", KEYWORD_IDENTITY),
        "{:a/b/c :namespace-slash, :a/b/c :name-slash}",
        "",
    );
    let text = fixture.own(
        "text",
        &[
            r#"(println "first\nsecond λ")
                (println nil ["one" nil {:two "三"}])
                (prn ["quote \"" "backslash \\"])
                nil"#,
            "#_42",
            "; only comment",
        ],
    );
    assert_eq!(text.status.code(), Some(0), "{text:?}");
    assert_eq!(text.stdout, b"nil\n");
    assert_eq!(
        text.stderr,
        "first\nsecond λ\nnil [one nil {:two 三}]\n[\"quote \\\"\" \"backslash \\\\\"]\n"
            .as_bytes()
    );
    success(&fixture.own("json", &["; comment", "#_42"]), "nil", "");
    success(
        &fixture.own("json", &[r#"(str "prefix" ["one" nil {:two "三"}])"#]),
        r#""prefix[\"one\" nil {:two \"三\"}]""#,
        "",
    );
}

#[test]
fn own_eval_reports_overflow_duplicate_keys_and_unsupported_input() {
    let fixture = Fixture::new("execution failures");
    for (source, expected) in [
        ("(+ 9223372036854775807 1 -1)", "long overflow"),
        ("(inc 9223372036854775807)", "long overflow"),
        ("(let [+ 42] (+ 1 2))", "cannot call +"),
        (DUPLICATE_MAP, "duplicate key: :a"),
        (r#"{"d" 1 (str "d") 2}"#, "duplicate key: d"),
        ("{nil 1 (if true nil) 2}", "duplicate key: null"),
        (
            r#"(inc 1 (println "arg before arity"))"#,
            "wrong number of args",
        ),
        ("(fn [value] value)", "unsupported"),
        ("(require 'clojure.set)", "unsupported"),
    ] {
        let output = fixture.own(
            "json",
            &[source, "(println \"unexpected later expression\")"],
        );
        let error = execution_error(&output);
        let message = error["message"].as_str().unwrap().to_ascii_lowercase();
        assert!(message.contains(expected), "{source}: {error}");
        let log = std::str::from_utf8(&output.stderr).unwrap();
        assert!(!log.contains("unexpected later expression"), "{log}");
        if source == DUPLICATE_MAP {
            assert!(log.starts_with(DUPLICATE_LOG), "{log}");
        }
        assert!(!log.lines().any(|line| line == "arg before arity"), "{log}");
        if source == "(fn [value] value)" {
            assert_eq!(error["path"], "<expr:1>");
            assert_eq!(error["line"], 1);
            assert_eq!(error["column"], 2);
        }
    }
}

#[test]
fn own_eval_bounds_collection_nesting_created_across_forms() {
    let fixture = Fixture::new("value depth");
    let source = format!("(def v nil) {} v", "(def v [v]) ".repeat(129));
    let result = fixture.own("json", &[&source]);
    let error = execution_error(&result);
    assert_eq!(error["message"], "Parenox value nesting limit exceeded");
}

#[test]
fn own_eval_runs_the_valid_read_prefix_and_does_not_merge_expression_args() {
    let fixture = Fixture::new("read boundaries");
    let prefix = fixture.own(
        "json",
        &[
            "(println \"before malformed λ\") (",
            "(println \"unexpected later expression\")",
        ],
    );
    execution_error(&prefix);
    let log = std::str::from_utf8(&prefix.stderr).unwrap();
    assert!(log.starts_with("before malformed λ\n"), "{log}");
    assert!(!log.contains("unexpected later expression"), "{log}");

    let split = fixture.own("json", &["(+ 1", "2)"]);
    assert_eq!(execution_error(&split)["path"], "<expr:1>");

    let unsupported = fixture.own("json", &["42", "\n  (require 'clojure.set)"]);
    let error = execution_error(&unsupported);
    assert_eq!(error["path"], "<expr:2>");
    assert_eq!(error["line"], 2);
    assert_eq!(error["column"], 4);
}

#[test]
fn own_backend_reports_run_and_test_as_unsupported_selections() {
    let fixture = Fixture::new("unsupported commands");
    let empty = fixture.0.join("empty");
    for (verb, args) in [
        ("run", &["missing.cljc"][..]),
        ("test", &["--format", "json", "portable.missing-test"][..]),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
            .current_dir(&fixture.0)
            .env_clear()
            .env("PATH", &empty)
            .args([verb, "--backend", "parenox"])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        let log = std::str::from_utf8(&output.stderr).unwrap();
        assert!(
            log.contains("--backend parenox currently supports eval"),
            "{log}"
        );
        if verb == "test" {
            let report = report(&output);
            assert_eq!(report["version"], 1);
            assert_eq!(report["ok"], false);
            assert_eq!(report["run_error"]["phase"], "selection");
        } else {
            assert!(output.stdout.is_empty(), "{output:?}");
        }
    }
}

#[test]
#[ignore = "requires the Clojure CLI and pinned JVM Clojure"]
fn own_eval_matches_the_pinned_jvm_for_implemented_forms() {
    let fixture = Fixture::new("JVM parity");
    for inputs in [
        STATE,
        READER_NAMESPACE,
        KEYWORD_IDENTITY,
        ORDER,
        DATA,
        &["(ns portable.vars) (def result 42)"],
        &["(ns portable.vars)"],
        &["; only comment", "#_42"],
    ] {
        let oracle = fixture.jvm("json", inputs);
        let own = fixture.own("json", inputs);
        assert_eq!(oracle.status.code(), Some(0), "{inputs:?}: {oracle:?}");
        assert_eq!(own.status, oracle.status, "{inputs:?}: {own:?}");
        assert_eq!(report(&own), report(&oracle), "{inputs:?}");
        assert_eq!(own.stderr, oracle.stderr, "{inputs:?}");
    }

    let inputs = &["(+ 9223372036854775807 1 -1)"];
    let oracle = execution_error(&fixture.jvm("json", inputs));
    let own = execution_error(&fixture.own("json", inputs));
    assert_eq!(own["message"], oracle["message"]);
}

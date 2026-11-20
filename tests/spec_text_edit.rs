use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const EDIT: &str =
    r#"{:new-text "🧪" :range {:start {:line 1N :character 2} :end {:line 3 :character 4}}}"#;
const EXTERNS: &str = "text-edit.edn";

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("parenox text edit {}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn cli(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_parenox"))
        .current_dir(directory)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn text_edit_contracts_round_trip_without_provider_sources() {
    let fixture = Fixture::new();
    let provider = fixture.0.join("provider");
    let consumer = fixture.0.join("consumer");
    fs::create_dir(&provider).unwrap();
    fs::create_dir(&consumer).unwrap();

    let (specs, _) = include_str!("fixtures/spec-text-edit.clj")
        .split_once("\n(def edit\n")
        .unwrap();
    let library = format!(
        "{specs}
        (defn accept {{:t/sig '[::text-edit :-> keyword?]}} [_] :accepted)
        (defn accept-all {{:t/sig '[::text-edits :-> keyword?]}} [_] :accepted)
        (defn normalized {{:t/sig '[:-> (s/coll-of ::text-edit :kind list?)]}} []
          (s/conform ::text-edits (list {EDIT})))"
    );
    let library_path = provider.join("edits.clj");
    fs::write(&library_path, library).unwrap();
    let exported = cli(
        &fixture.0,
        &["export-externs", "provider", "--format", "json"],
    );
    assert!(exported.status.success(), "{exported:?}");
    fs::write(provider.join(EXTERNS), &exported.stdout).unwrap();
    let validated = cli(
        &provider,
        &[
            "validate-externs",
            "edits.clj",
            "--externs",
            EXTERNS,
            "--format",
            "json",
        ],
    );
    assert!(validated.status.success(), "{validated:?}");
    let validated: Value = serde_json::from_slice(&validated.stdout).unwrap();
    assert_eq!(validated["complete"], true, "{validated}");
    fs::write(consumer.join(EXTERNS), exported.stdout).unwrap();
    fs::remove_file(library_path).unwrap();

    let checks = |source: &str, exit| {
        fs::write(consumer.join("client.clj"), source).unwrap();
        let outcome = cli(
            &consumer,
            &[
                "check",
                "client.clj",
                "--externs",
                EXTERNS,
                "--format",
                "json",
            ],
        );
        assert_eq!(outcome.status.code(), Some(exit), "{outcome:?}");
        let report: Value = serde_json::from_slice(&outcome.stdout).unwrap();
        assert_eq!(report["complete"], true, "{report}");
        assert_eq!(report["ok"], exit == 0, "{report}");
        report
    };

    let ns = "(ns consumer (:require [clojure.spec.alpha :as s] [demo.text-edit :as edit]))";
    checks(
        &format!(
            r#"{ns}
(defn submit {{:t/sig '[:-> keyword?]}} []
  (let [value {EDIT}]
    (edit/accept value)
    (edit/accept-all [value])
    (edit/accept-all (list value))
    (edit/accept-all (seq [value]))
    (edit/accept-all {{}})
    (edit/accept-all #{{}})
    (edit/accept-all (clojure.lang.MapEntry/create value value))
    (edit/accept-all (edit/normalized))))
(defn read-normalized {{:t/sig '[:-> (s/coll-of map? :kind list?)]}} []
  (edit/normalized))"#
        ),
        0,
    );

    let invalid = [
        format!("(edit/accept (dissoc {EDIT} :new-text))"),
        format!("(edit/accept (assoc {EDIT} :new-text 42))"),
        format!("(edit/accept (update {EDIT} :range dissoc :end))"),
        format!("(edit/accept-all (list (assoc-in {EDIT} [:range :start :line] 2147483648)))"),
        "(edit/accept-all [::s/invalid])".into(),
        format!("(edit/accept-all {{:entry {EDIT}}})"),
    ];
    for expression in invalid {
        let report = checks(
            &format!("{ns}\n(defn reject {{:t/sig '[:-> keyword?]}} [] {expression})"),
            1,
        );
        let [finding] = report["files"][0]["diagnostics"]
            .as_array()
            .unwrap()
            .as_slice()
        else {
            panic!("{expression}: {report}");
        };
        assert_eq!(finding["code"], "PNX2322", "{expression}: {finding}");
        assert_eq!(finding["contract"]["path"], EXTERNS, "{finding}");
    }
}

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("parenox finite int {}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn cli(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_parenox"))
            .current_dir(&self.0)
            .args(args)
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
fn finite_int_contracts_round_trip_through_the_cli() {
    let fixture = Fixture::new();
    let source = include_str!("fixtures/spec-enum-conformer.clj")
        .split_once(";; JVM-only assertions:")
        .unwrap()
        .0
        .replace(
            "(def file-change-type-enum",
            "(def ^:private file-change-type-enum",
        )
        + "(defn accept {:t/sig '[:file-event/type :-> keyword?]} [_] :accepted)\n";
    let export = |source: &str, snapshot: &str| {
        fs::write(fixture.0.join("provider.clj"), source).unwrap();
        let exported = fixture.cli(&["export-externs", "provider.clj", "--format", "json"]);
        assert!(exported.status.success(), "{exported:?}");
        fs::write(fixture.0.join(snapshot), exported.stdout).unwrap();
        let validated = fixture.cli(&[
            "validate-externs",
            "provider.clj",
            "--externs",
            snapshot,
            "--format",
            "json",
        ]);
        assert!(validated.status.success(), "{validated:?}");
        let report: Value = serde_json::from_slice(&validated.stdout).unwrap();
        assert_eq!(report["complete"], true, "{report}");
    };

    export(&source, "before.edn");
    export(&source.replace(" 2 :changed 3 :deleted", ""), "after.edn");
    let stale = fixture.cli(&[
        "validate-externs",
        "provider.clj",
        "--externs",
        "before.edn",
        "--format",
        "json",
    ]);
    assert_eq!(stale.status.code(), Some(1), "{stale:?}");
    let report: Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(report["complete"], true, "{report}");
    assert_eq!(report["files"][1]["diagnostics"][0]["code"], "PNX2322");
    fs::remove_file(fixture.0.join("provider.clj")).unwrap();

    let diff = fixture.cli(&[
        "diff-externs",
        "--before",
        "before.edn",
        "--after",
        "after.edn",
        "--json",
    ]);
    assert_eq!(diff.status.code(), Some(1), "{diff:?}");
    let report: Value = serde_json::from_slice(&diff.stdout).unwrap();
    assert_eq!(report["complete"], true, "{report}");
    assert_eq!(report["compatible"], false, "{report}");
    let [change] = report["changes"].as_array().unwrap().as_slice() else {
        panic!("{report}");
    };
    assert_eq!(change["name"], "lsp4clj.coercer/accept");
    assert_eq!(change["kind"], "breaking");
    assert_eq!(change["argument_count"], 1);

    let reverse = fixture.cli(&[
        "diff-externs",
        "--before",
        "after.edn",
        "--after",
        "before.edn",
        "--json",
    ]);
    assert!(reverse.status.success(), "{reverse:?}");
    let report: Value = serde_json::from_slice(&reverse.stdout).unwrap();
    assert_eq!(report["complete"], true, "{report}");

    for (snapshot, value, valid) in [
        ("before.edn", "1", true),
        ("before.edn", "2", true),
        ("before.edn", "1N", false),
        ("before.edn", "4", false),
        ("after.edn", "1", true),
        ("after.edn", "2", false),
    ] {
        fs::write(
            fixture.0.join("consumer.clj"),
            format!(
                "(ns consumer (:require [lsp4clj.coercer :as c]))
                     (defn pick {{:t/sig '[:-> keyword?]}} [] (c/accept {value}))"
            ),
        )
        .unwrap();
        let checked = fixture.cli(&[
            "check",
            "consumer.clj",
            "--externs",
            snapshot,
            "--format",
            "json",
        ]);
        assert_eq!(
            checked.status.code(),
            Some(u8::from(!valid).into()),
            "{checked:?}"
        );
        let report: Value = serde_json::from_slice(&checked.stdout).unwrap();
        assert_eq!(report["complete"], true, "{report}");
        assert_eq!(report["ok"], valid, "{report}");
        if !valid {
            let [finding] = report["files"][0]["diagnostics"]
                .as_array()
                .unwrap()
                .as_slice()
            else {
                panic!("{report}");
            };
            assert_eq!(finding["code"], "PNX2322");
            assert_eq!(finding["contract"]["path"], snapshot);
        }
    }
}

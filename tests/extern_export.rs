use std::process::Command;

use parenox::{ExternChangeKind, ExternSnapshot, Session, Source, compare_externs};

const SHOWCASE: Source<'_> = Source {
    path: "examples/showcase.clj",
    source: include_str!("../examples/showcase.clj"),
};

fn export(sources: &[Source<'_>]) -> String {
    let export = Session::default().export_externs(sources, &[]);
    export.edn.unwrap_or_else(|| panic!("{:#?}", export.report))
}

#[test]
fn exported_showcase_checks_a_consumer_without_library_sources_or_named_specs() {
    let edn = export(&[SHOWCASE]);
    assert!(!edn.contains("::") && !edn.contains("demo.showcase/require-tenant"));
    let externs = [Source {
        path: "showcase.edn",
        source: &edn,
    }];
    let mut session = Session::default();
    let validated = session.validate_externs(&[SHOWCASE], &externs);
    assert!(validated.ok && validated.complete, "{validated:#?}");

    let client = include_str!("../examples/showcase_client.clj");
    let valid = session.check_project_with_externs(
        &[Source {
            path: "client.clj",
            source: client,
        }],
        &externs,
    );
    assert!(valid.ok && valid.complete, "{valid:#?}");

    let broken = client.replace(":trace-id \"run-42\"", ":weights [\"heavy\"]");
    let invalid = session.check_project_with_externs(
        &[Source {
            path: "client.clj",
            source: &broken,
        }],
        &externs,
    );
    let [diagnostic] = invalid.files[0].diagnostics.as_slice() else {
        panic!("{invalid:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(
        diagnostic.contract.as_ref().unwrap().path.as_deref(),
        Some("showcase.edn")
    );
    assert!(diagnostic.actual.as_ref().unwrap().contains(":weights"));
}

#[test]
fn export_inlines_named_specs_and_reports_optional_to_required_as_breaking() {
    let shape = "(ns shape (:require [clojure.spec.alpha :as s]))
        (s/def ::label (s/nilable string?))
        (s/def ::item (s/keys :opt-un [::label]))";
    let required = shape.replace(":opt-un", ":req-un");
    let api = Source {
        path: "api.clj",
        source: "(ns api (:require [shape :as data]))
            (defn label {:t/sig '[::data/item :-> string?]} [item]
              (or (:label item) \"default\"))",
    };
    let specs = Source {
        path: "shape.clj",
        source: shape,
    };
    let before = export(&[api, specs]);
    let after = export(&[
        api,
        Source {
            source: &required,
            ..specs
        },
    ]);
    let reference = export(&[specs, api]);
    assert_eq!(before, reference);
    let diff = compare_externs(
        ExternSnapshot {
            sources: &[],
            externs: &[Source {
                path: "before.edn",
                source: &before,
            }],
        },
        ExternSnapshot {
            sources: &[],
            externs: &[Source {
                path: "after.edn",
                source: &after,
            }],
        },
    );
    assert!(diff.complete && !diff.compatible, "{diff:#?}");
    let [change] = diff.changes.as_slice() else {
        panic!("{diff:#?}")
    };
    assert_eq!(change.kind, ExternChangeKind::Breaking);
    assert_eq!(change.name, "api/label");
}

#[test]
fn deeply_expanded_specs_cannot_produce_an_unreadable_export() {
    use std::fmt::Write;

    let mut specs = String::from(
        "(ns shapes (:require [clojure.spec.alpha :as s]))\n(s/def ::level0 string?)\n",
    );
    for level in 1..=45 {
        writeln!(
            specs,
            "(s/def ::level{level} (s/keys :req-un [::level{}]))",
            level - 1
        )
        .unwrap();
    }
    let specs = Source {
        path: "shapes.clj",
        source: &specs,
    };
    let library = "\
(ns lib (:require [shapes :as shapes]))
(def status :ready)
(defn too-deep {:t/sig '[::shapes/level45 :-> nil?]} [_] nil)";
    let candidate = Source {
        path: "lib.clj",
        source: library,
    };
    let mut session = Session::default();
    let report = session.check_project_with_externs(&[candidate, specs], &[]);
    assert!(report.ok && report.complete, "{report:#?}");
    let failed = session.export_externs(&[candidate, specs], &[]);
    assert!(failed.edn.is_none() && !failed.report.ok && !failed.report.complete);
    let [error] = failed.report.files[0].diagnostics.as_slice() else {
        panic!("{failed:#?}");
    };
    assert_eq!(error.code, "PNX9012");
    assert_eq!(error.span.start.line, 3);
    assert!(error.id.is_some());
    assert!(error.message.contains("Cannot export lib/too-deep"));
    assert!(error.message.contains("Reader nesting limit"));
    assert!(failed.report.files[1].ok);

    let fixed = library.replace("level45", "level30");
    let repaired = session.export_externs(
        &[
            Source {
                source: &fixed,
                ..candidate
            },
            specs,
        ],
        &[],
    );
    let edn = repaired
        .edn
        .unwrap_or_else(|| panic!("{:#?}", repaired.report));
    assert!(edn.contains("lib/status {:value :ready}") && edn.contains("lib/too-deep"));
    let reloaded = Session::default().check_project_with_externs(
        &[],
        &[Source {
            path: "api.edn",
            source: &edn,
        }],
    );
    assert!(reloaded.ok && reloaded.complete, "{reloaded:#?}");
}

#[test]
fn export_preserves_bounded_binders_arities_and_literal_data() {
    let source = Source {
        path: "library.clj",
        source: r#"(ns library (:require [clojure.spec.alpha :as s]))
            (def ^:private secret 42)
            (defn- hidden {:t/sig '[:-> number?]} [] secret)
            (defn same {:t/sig '(t/all [(x :< number?)] [x :-> x])} [x] x)
            (defn values
              {:t/sig '[[{:literal #{"a\"🙂\n\b\f\u0000" \λ true ::tag}} :-> number?]
                        [number? & (s/tuple char? (s/coll-of ::tag :kind set?)) :-> number?]]}
              ([item] (hidden))
              ([value & _] value))
            (s/def ::tag #{::tag})
            (defn collect
              {:t/sig '[(s/nilable (s/coll-of number? :kind seq?))
                        :-> (s/nilable (s/coll-of number? :kind seq?))]}
              [xs] xs)"#,
    };
    let edn = export(&[source]);
    assert!(!edn.contains("library/hidden") && !edn.contains("library/secret"));
    let exported = Source {
        path: "library.edn",
        source: &edn,
    };
    let report = Session::default().validate_externs(&[source], &[exported]);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn incomplete_check_or_reserved_name_produces_no_partial_snapshot() {
    let good = Source {
        path: "ok.clj",
        source: "(ns ok) (defn ok {:t/sig '[:-> number?]} [] 0)",
    };
    let suppressed = Source {
        path: "ignored.clj",
        source: "(ns ignored)
              ;; parenox-ignore PNX2322: legacy caller
              (defn bad {:t/sig '[:-> number?]} [] \"not a number\")",
    };
    let reserved = Source {
        path: "core.clj",
        source: "(ns clojure.core)
              (defn inc {:t/sig '[number? :-> number?]} [x] x)",
    };
    let mut session = Session::default();
    let incomplete = session.export_externs(&[good, suppressed], &[]);
    assert!(incomplete.edn.is_none() && incomplete.report.ok && !incomplete.report.complete);
    let invalid = session.export_externs(&[good, reserved], &[]);
    assert!(invalid.edn.is_none() && !invalid.report.ok && !invalid.report.complete);
    let [error] = invalid.report.files[1].diagnostics.as_slice() else {
        panic!("{invalid:#?}");
    };
    assert_eq!(error.code, "PNX9012");
    assert!(error.message.contains("clojure.core/inc") && error.id.is_some());
    let reduced = session.export_externs(&[], &[]);
    assert_eq!(reduced.edn.as_deref(), Some("{}\n"));
}

#[test]
fn map_shape_options_reject_invalid_and_overlapping_fields() {
    for shape in [
        "(t/map {:req {:x number?} :opt {:x number?}})",
        "(t/map {:opt {:x number?} :extra :assume-open})",
        "(t/map {:req [:x]})",
    ] {
        let source = format!("(ns map-api) (defn label {{:t/sig '[{shape} :-> number?]}} [row] 0)");
        let report = Session::default().check("map-api.clj", &source);
        assert!(!report.ok && !report.complete, "{report:#?}");
        assert_eq!(report.diagnostics[0].code, "PNX9001");
    }
}

#[test]
fn closed_optional_string_field_survives_export_without_becoming_nilable() {
    let library = Source {
        path: "labels.clj",
        source: r#"(ns labels)
            (defn label {:t/sig '[(t/map {:opt {"display" string?}}) :-> string?]} [row]
              (or (get row "display") "default"))"#,
    };
    let edn = export(&[library]);
    let snapshot = Source {
        path: "labels.edn",
        source: &edn,
    };
    let mut session = Session::default();
    for (arg, valid) in [
        (r#"{}"#, true),
        (r#"{"display" "Ada"}"#, true),
        (r#"{"display" nil}"#, false),
        (r#"{:unexpected "Ada"}"#, false),
    ] {
        let source = format!(
            "(ns client (:require [labels :as x])) (defn render {{:t/sig '[:-> string?]}} [] (x/label {arg}))"
        );
        let report = session.check_project_with_externs(
            &[Source {
                path: "client.clj",
                source: &source,
            }],
            &[snapshot],
        );
        assert!(report.complete, "{report:#?}");
        assert_eq!(report.ok, valid, "{arg}: {report:#?}");
        if !valid {
            let [diagnostic] = report.files[0].diagnostics.as_slice() else {
                panic!("{report:#?}");
            };
            assert_eq!(diagnostic.code, "PNX2322");
        }
    }
}

#[test]
fn cli_exports_only_library_functions_and_puts_failures_on_stderr() {
    let cli = || Command::new(env!("CARGO_BIN_EXE_parenox"));
    let output = cli()
        .args([
            "export-externs",
            "--config=examples/split_externs/parenox.edn",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("{demo.split-externs/patient-label") && stdout.ends_with("}\n"));
    assert!(stdout.contains("vendor.clinic/find-patient"));
    assert!(!stdout.contains("clojure.string/starts-with?"));

    let output = cli()
        .args([
            "export-externs",
            SHOWCASE.path,
            "examples/showcase_bad.clj",
            "--format=json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(
        report["files"][1]["diagnostics"].as_array().unwrap().len(),
        6
    );
}

use parenox_analysis::{Report, Session, Source};

const VENDOR: Source = Source {
    path: "vendor.edn",
    source: "{vendor/label [string? :-> string?]}",
};

fn app_report(report: &parenox_analysis::ProjectReport) -> &Report {
    report
        .files
        .iter()
        .find(|file| file.path == "app_test.clj")
        .unwrap()
}

fn check_with_vendor(source: &str) -> parenox_analysis::ProjectReport {
    Session::default().check_project_with_externs(
        &[Source {
            path: "app_test.clj",
            source,
        }],
        &[VENDOR],
    )
}

#[test]
fn checks_standard_literal_and_predicate_assertions_without_reporting_test_failures() {
    let source = r#"
(ns demo.checked-test
  (:require [clojure.test :as t :refer [deftest is testing]]))

(deftest scalars
  (testing "literal assertions"
    (is nil)
    (is false "this would fail only at runtime")
    (is true)
    (is 42)
    (is 1/2)
    (is :value)
    (is "value"))
  (t/testing "standard predicates"
    (t/is (= 1 2))
    (t/is (clojure.core/= :value :value))
    (is (nil? nil))
    (is (clojure.core/some? false))
    (is (true? true))
    (is (clojure.core/false? false))
    (is (not false))))
(clojure.test/deftest qualified
  (clojure.test/is (clojure.core/not true)))
"#;
    let report = Session::default().check("app_test.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    assert!(report.diagnostics.is_empty(), "{report:#?}");
}

#[test]
fn two_assertions_report_distinct_argument_occurrences() {
    let source = r#"
(ns demo.checked-test (:require [clojure.test :as t] [vendor :as v]))

(t/deftest duplicate-diagnostics
  (t/is (= "label" (v/label nil)))
  (t/testing "nested"
    (t/is (some? (v/label nil)) "same contract, different source occurrence")))
"#;
    let checked = check_with_vendor(source);
    let report = app_report(&checked);
    assert!(report.complete && !report.ok, "{checked:#?}");
    let [first, second] = report.diagnostics.as_slice() else {
        panic!("{checked:#?}");
    };
    for finding in [first, second] {
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            "nil"
        );
        let contract = finding.contract.as_ref().unwrap();
        assert_eq!(contract.name, "vendor/label");
        assert_eq!(contract.path.as_deref(), Some(VENDOR.path));
    }
    assert_ne!(first.span, second.span);
    assert_ne!(first.id.as_deref().unwrap(), second.id.as_deref().unwrap());
}

#[test]
fn unsupported_assertions_keep_the_deftest_gap_and_only_proven_errors() {
    for unsupported in [
        "(t/is (= true (opaque/value)))",
        "(t/is (thrown? Exception 1))",
        "(t/are [x] (= x x) :value)",
        "(t/is true (str \"computed message\"))",
        "(t/is)",
    ] {
        let source = format!(
            r#"
(ns demo.checked-test
  (:require [clojure.test :as t] [vendor :as v] [opaque :as opaque]))

(t/deftest covered (t/is false))
(t/deftest partial
  (t/is (= "label" (v/label nil)))
  {unsupported})
"#
        );
        let checked = check_with_vendor(&source);
        let report = app_report(&checked);
        assert!(!report.complete, "{unsupported}: {checked:#?}");
        let [gap, mismatch] = report.diagnostics.as_slice() else {
            panic!("{unsupported}: {checked:#?}");
        };
        assert_eq!(gap.code, "PNX9001", "{gap:#?}");
        assert_eq!(gap.severity, "warning", "{gap:#?}");
        assert_eq!(
            gap.span.start.offset,
            source.find("(t/deftest partial").unwrap() + 1
        );
        assert_eq!(mismatch.code, "PNX2322", "{mismatch:#?}");
        assert_eq!(
            &source[mismatch.span.start.offset..mismatch.span.end.offset],
            "nil"
        );
    }
}

#[test]
fn shadowing_and_assert_expr_extensions_keep_assertions_opaque() {
    for (source, replacement) in [
        (
            r#"
(ns demo.checked-test (:require [clojure.test :as t] [vendor :as v]))
(t/deftest replaced (t/is (= "label" (v/label nil))))
"#,
            Some(Source {
                path: "clojure/test.clj",
                source: r#"
(ns clojure.test)
(defmacro deftest [name & body] `(defn ~name [] nil))
(defmacro is [form] nil)
"#,
            }),
        ),
        (
            r#"
(ns demo.checked-test
  (:require [clojure.test :as t] [vendor :as v] [demo.assertion-extension]))
(t/deftest custom (t/is (= "label" (v/label nil))))
"#,
            Some(Source {
                path: "demo/assertion_extension.clj",
                source: r#"
(ns demo.assertion-extension (:require [clojure.test :as t]))
(defmethod t/assert-expr '= [message form] nil)
"#,
            }),
        ),
        (
            r#"
(ns demo.checked-test
  (:refer-clojure :exclude [nil?])
  (:require [clojure.test :as t] [vendor :as v]))
(defn nil? {:t/sig '[any? :-> boolean?]} [_] true)
(t/deftest shadowed (t/is (nil? (v/label nil))))
"#,
            None,
        ),
    ] {
        let mut sources = vec![Source {
            path: "app_test.clj",
            source,
        }];
        sources.extend(replacement);
        let checked = Session::default().check_project_with_externs(&sources, &[VENDOR]);
        let report = app_report(&checked);
        assert!(!report.complete, "{checked:#?}");
        assert!(!report.diagnostics.is_empty(), "{checked:#?}");
        assert!(
            report
                .diagnostics
                .iter()
                .all(|finding| finding.code == "PNX9001"),
            "{checked:#?}"
        );
    }
}

#[test]
fn a_warm_extern_edit_rechecks_an_unchanged_deftest() {
    let app = Source {
        path: "app_test.clj",
        source: r#"
(ns demo.checked-test (:require [clojure.test :as t] [vendor :as v]))
(t/deftest vendor-contract (t/is (= "label" (v/label nil))))
"#,
    };
    let accepts_nil = Source {
        source: "{vendor/label [(clojure.spec.alpha/nilable string?) :-> string?]}",
        ..VENDOR
    };
    let mut session = Session::default();
    for _ in 0..2 {
        let checked = session.check_project_with_externs(&[app], &[accepts_nil]);
        assert!(checked.ok && checked.complete, "{checked:#?}");
    }

    let changed = session.check_project_with_externs(&[app], &[VENDOR]);
    let report = app_report(&changed);
    assert!(report.complete && !report.ok, "{changed:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{changed:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert_eq!(
        &app.source[finding.span.start.offset..finding.span.end.offset],
        "nil"
    );
    assert_eq!(
        finding.contract.as_ref().unwrap().path.as_deref(),
        Some(VENDOR.path)
    );

    let warm = session.check_project_with_externs(&[app], &[VENDOR]);
    assert_eq!(app_report(&warm).diagnostics[0].id, finding.id, "{warm:#?}");
    let restored = session.check_project_with_externs(&[app], &[accepts_nil]);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn an_unproven_operand_compatibility_keeps_the_coverage_gap() {
    let app = Source {
        path: "app_test.clj",
        source: r#"
(ns demo.checked-test (:require [clojure.test :as t] [vendor :as v] [clojure.string :as str]))
(t/deftest unchecked-input (t/is (= "label" (str/trim (v/value)))))
"#,
    };
    let externs = Source {
        source: "{vendor/value [:-> any?]}",
        ..VENDOR
    };
    let checked = Session::default().check_project_with_externs(&[app], &[externs]);
    let report = app_report(&checked);
    assert!(!report.complete, "{checked:#?}");
    assert_eq!(report.diagnostics.len(), 1, "{checked:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001", "{checked:#?}");
}

#[test]
fn covering_a_test_rechecks_a_function_whose_unknown_type_was_already_masked() {
    let opaque = r#"
(ns demo.checked-test (:require [clojure.test :as t] [vendor :as v] [clojure.string :as str]))
(defn example {:t/sig '[:-> string?]} [] (str/trim (v/value)))
(t/deftest checking (t/is (thrown? Exception 0)))
"#;
    let covered = opaque.replace("(thrown? Exception 0)", "false");
    let externs = Source {
        source: "{vendor/value [:-> any?]}",
        ..VENDOR
    };
    let mut session = Session::default();
    for (source, expected) in [
        (opaque, "PNX9001"),
        (&covered, "PNX9004"),
        (opaque, "PNX9001"),
    ] {
        let app = Source {
            path: "app_test.clj",
            source,
        };
        let checked = session.check_project_with_externs(&[app], &[externs]);
        let report = app_report(&checked);
        assert!(!report.complete, "{checked:#?}");
        assert_eq!(report.diagnostics.len(), 1, "{checked:#?}");
        assert_eq!(report.diagnostics[0].code, expected, "{checked:#?}");
        assert_eq!(report.stats.checked_functions, 1, "{checked:#?}");
    }
}

#[test]
fn checked_assertions_preserve_source_reference_findings_when_externs_disagree() {
    let app = Source {
        path: "app_test.clj",
        source: r#"
(ns demo.checked-test (:require [clojure.test :as t] [vendor :as v]))
(t/deftest wrong-source-arity (t/is (= (v/label "a" "b") "")))
(t/deftest missing-source-var (t/is (= (v/missing) "")))
"#,
    };
    let vendor = Source {
        path: "vendor.clj",
        source: "(ns vendor)\n(defn label [x] x)",
    };
    let externs = Source {
        source: "{vendor/label [string? string? :-> string?] vendor/missing [:-> string?]}",
        ..VENDOR
    };
    let checked = Session::default().check_project_with_externs(&[app, vendor], &[externs]);
    let report = app_report(&checked);
    let codes: Vec<_> = report
        .diagnostics
        .iter()
        .map(|item| item.code.as_str())
        .collect();
    assert_eq!(codes, ["PNX2554", "PNX2304"], "{checked:#?}");
}

#[test]
fn quoted_assertion_extensions_and_prior_namespace_updates_keep_test_bodies_opaque() {
    for setup in [
        "(def extension (resolve 't/assert-expr))\n(.addMethod ^clojure.lang.MultiFn @extension '= (fn [_ _] true))",
        "(ns-unalias *ns* 't)\n(alias 't 'other.test)",
    ] {
        let source = format!(
            "(ns demo.checked-test (:require [clojure.test :as t] [clojure.string :as str]))
{setup}
(t/deftest hidden (t/is (= \"\" (str/trim 42))))"
        );
        let report = Session::default().check("app_test.clj", &source);
        assert!(!report.complete, "{report:#?}");
        assert!(report.diagnostics.iter().all(|item| item.code != "PNX2322"));
        assert!(report.diagnostics.iter().any(|item| {
            item.message
                .contains("Type checking for t/deftest declarations is unavailable")
        }));
    }
}

#[test]
fn test_vars_replacing_source_definitions_or_builtins_keep_the_declaration_gap() {
    for (definitions, name) in [
        (
            "(defn label {:t/sig '[:-> string?]} [] \"example\")",
            "label",
        ),
        ("", "str"),
        ("", "defn"),
        ("(t/deftest repeated (t/is true))", "repeated"),
    ] {
        let source = format!(
            "(ns demo.checked-test (:require [clojure.test :as t]))
{definitions}
(t/deftest {name} (t/is true))"
        );
        let report = Session::default().check("app_test.clj", &source);
        assert!(!report.complete, "{name}: {report:#?}");
        assert!(report.diagnostics.iter().any(|item| item.code == "PNX9001"));
    }
}

#[test]
fn an_unexecuted_type_comparison_does_not_block_a_checked_assertion() {
    let app = Source {
        path: "app_test.clj",
        source: r#"
(ns demo.checked-test (:require [clojure.test :as t] [vendor :as v] [clojure.string :as str]))
(t/deftest unreachable-mismatch
  (t/is (= :ok (if true :ok (str/upper-case (v/value))))))
"#,
    };
    let externs = Source {
        source: "{vendor/value [:-> any?]}",
        ..VENDOR
    };
    let checked = Session::default().check_project_with_externs(&[app], &[externs]);
    assert!(checked.ok && checked.complete, "{checked:#?}");
}

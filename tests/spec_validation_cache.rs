use parenox::{Session, Source};

#[test]
fn ambiguous_key_options_do_not_supply_runtime_validation_facts() {
    for (spec, expected) in [
        (
            "(s/keys :opt-un [:second/title] :req-un [:first/title])",
            "Repeated field",
        ),
        (
            "(s/keys :req-un [:first/title] :req-un [:second/other])",
            "Repeated s/keys directive",
        ),
    ] {
        let source = format!(
            r#"(ns demo (:require [clojure.spec.alpha :as s]))
(s/def :first/title string?)
(s/def :second/title number?)
(s/def :second/other keyword?)
(s/def ::row {spec})
(defn checked {{:t/sig '[map? :-> string?]}} [row]
  (if (s/valid? ::row row) (:title row) "fallback"))
"#
        );
        let report = Session::default().check("demo.clj", &source);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.message.contains(expected)),
            "{report:?}"
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.message.contains("cannot refine unchecked spec :demo/row")),
            "{report:?}"
        );
    }
}

#[test]
fn unqualified_spec_keys_keep_the_full_local_name_after_validation() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
(s/def :wire/nested/title string?)
(s/def ::row (s/keys :req-un [:wire/nested/title]))
(defn checked {:t/sig '[map? :-> string?]} [row]
  (if (s/valid? ::row row) (:nested/title row) "fallback"))
"#;
    let report = Session::default().check("demo.clj", source);
    assert!(report.ok && report.complete, "{report:?}");
}

#[test]
fn removing_supported_conversion_evidence_invalidates_an_equal_input_shape() {
    let provider =
        |spec| format!("(ns wire (:require [clojure.spec.alpha :as s]))\n(s/def ::line {spec})");
    let exact = "(s/and integer? (s/conformer int))";
    let plain = provider(exact);
    // The TypeReader can still infer the successful input domain of the inner
    // spec. The wrapped callback does not have modeled conversion evidence.
    let wrapped = provider(&format!("(s/with-gen {exact} (fn [] nil))"));
    let consumer = r#"(ns demo (:require [clojure.spec.alpha :as s]))
(defn line {:t/sig '[:wire/line :-> :wire/line]} [x] (s/conform :wire/line x))
(defn stable {:t/sig '[:wire/line :-> :wire/line]} [x] x)
"#;
    let mut session = Session::default();
    for (spec, complete, reused) in [(&plain, true, 0), (&wrapped, false, 1), (&plain, true, 1)] {
        let report = session.check_project(
            &[
                Source {
                    path: "demo.clj",
                    source: consumer,
                },
                Source {
                    path: "wire.clj",
                    source: spec,
                },
            ],
            None,
        );
        let file = &report.files[0];
        assert_eq!(file.complete, complete, "{file:?}");
        assert_eq!(file.stats.reused_functions, reused, "{file:?}");
        if !complete {
            assert!(
                file.diagnostics
                    .iter()
                    .any(|d| d.message.contains("supported conversion")),
                "{file:?}"
            );
        }
    }
}

#[test]
fn exported_contracts_preserve_the_checked_int_range() {
    let source = r#"(ns wire (:require [clojure.spec.alpha :as s]))
(s/def ::line (s/and integer? (s/conformer int)))
(defn line {:t/sig '[::line :-> ::line]} [x] x)
"#;
    let export = Session::default().export_externs(
        &[Source {
            path: "wire.clj",
            source,
        }],
        &[],
    );
    let edn = export.edn.unwrap_or_else(|| panic!("{:?}", export.report));
    let consumer = r#"(ns demo (:require [wire]))
(defn copy {:t/sig '[:-> number?]} [] (wire/line 2147483648))
"#;
    let report = Session::default().check_project(
        &[Source {
            path: "demo.clj",
            source: consumer,
        }],
        Some(Source {
            path: "wire.edn",
            source: &edn,
        }),
    );
    let [error] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:?}");
    };
    assert_eq!(error.code, "PNX2322");
}

#[test]
fn body_only_spec_changes_recheck_transitive_callers_but_not_unrelated_functions() {
    let consumer = r#"(ns consumer (:require [clojure.spec.alpha :as s]))
(defn- title [row]
  (if (s/valid? :wire/row row) (:title row) ""))
(defn render {:t/sig '[map? :-> string?]} [row]
  (title row))
(defn untouched {:t/sig '[:-> boolean?]} [] true)
"#;
    let provider = r#"(ns wire (:require [clojure.spec.alpha :as s]))
(s/def ::title string?)
(s/def ::row (s/keys :req-un [::title]))
"#;
    let changed = provider.replace("string?", "number?");
    let missing = "(ns wire)";
    let mut session = Session::default();
    for (source, mismatch, unavailable) in [
        (provider, false, false),
        (&changed, true, false),
        (missing, false, true),
        (provider, false, false),
    ] {
        let input = [
            Source {
                path: "consumer.clj",
                source: consumer,
            },
            Source {
                path: "wire.clj",
                source,
            },
        ];
        let warm = session.check_project(&input, None);
        let cold = Session::default().check_project(&input, None);
        let report = &warm.files[0];
        assert_eq!(
            serde_json::to_value(&report.diagnostics).unwrap(),
            serde_json::to_value(&cold.files[0].diagnostics).unwrap()
        );
        assert_eq!(
            report.diagnostics.iter().any(|d| d.code == "PNX2322"),
            mismatch,
            "{report:?}"
        );
        assert_eq!(
            report
                .diagnostics
                .iter()
                .any(|d| d.message.contains(":wire/row")),
            unavailable,
            "{report:?}"
        );
        if !mismatch && !unavailable {
            assert!(report.ok && report.complete, "{report:?}");
        }
        assert_eq!(
            report.stats.checked_functions + report.stats.reused_functions,
            2
        );
        if report.stats.reused_parses == 1 {
            assert_eq!(report.stats.checked_functions, 1, "{report:?}");
            assert_eq!(report.stats.reused_functions, 1, "{report:?}");
        }
    }
}

use parenox_analysis::{Session, Source};

const FOLDING_RANGES: &str = include_str!("../../../tests/fixtures/spec-folding-ranges.clj");

#[test]
fn named_keyword_specs_and_aliases_reject_the_marker() {
    let source = r#"
(ns demo.keyword-spec (:require [clojure.spec.alpha :as s]))
(s/def ::label keyword?)
(s/def ::alias ::label)

(defn valid-label {:t/sig '[:-> #{true}]} []
  (s/valid? ::label :region))
(defn invalid-label {:t/sig '[:-> #{false}]} []
  (s/valid? ::label ::s/invalid))
(defn invalid-alias {:t/sig '[:-> #{false}]} []
  (s/valid? ::alias ::s/invalid))
(defn conformed-label {:t/sig '[:-> #{:region}]} []
  (s/conform ::label :region))
(defn conformed-alias {:t/sig '[:-> #{:region}]} []
  (s/conform ::alias :region))
(defn conformed-invalid {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::alias ::s/invalid))
(defn guarded {:t/sig '[keyword? :-> (s/nilable ::label)]} [x]
  (when (s/valid? ::alias x) x))
(defn raw-keyword {:t/sig '[:-> #{true}]} []
  (keyword? ::s/invalid))
"#;
    let report = Session::default().check("keyword_spec.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn keyword_conjunction_preserves_the_input_in_both_orders() {
    for stages in ["keyword? any?", "any? keyword?"] {
        let source = format!(
            r#"
(ns demo.keyword-conjunction (:require [clojure.spec.alpha :as s]))
(s/def ::label (s/and {stages}))
(defn valid-label {{:t/sig '[:-> #{{true}}]}} []
  (s/valid? ::label :region))
(defn invalid-label {{:t/sig '[:-> #{{false}}]}} []
  (s/valid? ::label ::s/invalid))
(defn conformed-label {{:t/sig '[:-> #{{:region}}]}} []
  (s/conform ::label :region))
(defn conformed-invalid {{:t/sig '[:-> #{{::s/invalid}}]}} []
  (s/conform ::label ::s/invalid))
"#
        );
        let report = Session::default().check("keyword_conjunction.clj", &source);
        assert!(report.ok && report.complete, "{stages}: {report:#?}");
    }
}

#[test]
fn folding_ranges_reject_only_the_marker_at_the_registered_field() {
    let specs = FOLDING_RANGES.split_once(";; JVM-only").unwrap().0;
    let valid_uses = r#"
(defn folding {:t/sig '[::folding-ranges :-> keyword?]} [_] :accepted)
(defn ordinary-map {:t/sig '[(t/map {:req {:kind keyword?}}) :-> keyword?]} [_] :accepted)
(defn sends {:t/sig '[:-> keyword?]} []
  (folding [{:start-line ::s/invalid :end-line false}])
  (folding [{:start-line nil :end-line ::s/invalid :collapsed-text ::s/invalid :kind :region}])
  (ordinary-map {:kind ::s/invalid}))
"#;
    let mut session = Session::default();
    let source = format!("{specs}{valid_uses}");
    let report = session.check("folding.clj", &source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 5, "{report:#?}");
    assert!(report.diagnostics.iter().all(|finding| {
        finding.code == "PNX9001" && finding.message.contains("checks presence only")
    }));

    let source = format!(
        "{source}\n(defn sends-marker {{:t/sig '[:-> keyword?]}} []\n  (folding [(assoc {{:start-line nil :end-line false :kind :region}} :kind ::s/invalid)]))"
    );
    let report = session.check("folding.clj", &source);
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.severity == "error")
        .collect();
    let [finding] = errors.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert!(finding.message.contains(":clojure.spec.alpha/invalid"));
}

#[test]
fn keyword_refinement_survives_export_and_import_without_source() {
    let provider = r#"
(ns demo.exported-keyword (:require [clojure.spec.alpha :as s]))
(s/def ::line integer?)
(s/def ::kind clojure.core/keyword?)
(s/def ::row (s/keys :req-un [::line] :opt-un [::kind]))
(s/def ::rows (s/coll-of ::row))
(defn accept {:t/sig '[::rows :-> keyword?]} [_] :accepted)
"#;
    let export = Session::default().export_externs(
        &[Source {
            path: "provider.clj",
            source: provider,
        }],
        &[],
    );
    assert!(export.report.ok && export.report.complete, "{export:#?}");
    let edn = export.edn.unwrap();
    let check = |body| {
        let consumer = format!(
            "(ns demo.consumer (:require [clojure.spec.alpha :as s] [demo.exported-keyword :as wire]))\n{body}"
        );
        Session::default().check_project_with_externs(
            &[Source {
                path: "consumer.clj",
                source: &consumer,
            }],
            &[Source {
                path: "provider.edn",
                source: &edn,
            }],
        )
    };
    let accepted = check(
        r#"(defn sends {:t/sig '[:-> keyword?]} []
             (wire/accept [{:line 1}])
             (wire/accept [{:line 1 :kind :region}]))"#,
    );
    assert!(accepted.ok && accepted.complete, "{accepted:#?}");

    for (body, code) in [
        (
            r#"(defn sends {:t/sig '[:-> keyword?]} []
                 (wire/accept [{:line 1 :kind ::s/invalid}]))"#,
            "PNX2322",
        ),
        (
            r#"(defn sends {:t/sig '[keyword? :-> keyword?]} [kind]
                 (wire/accept [{:line 1 :kind kind}]))"#,
            "PNX9004",
        ),
    ] {
        let report = check(body);
        let [finding] = report.files[0].diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, code, "{finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().path.as_deref(),
            Some("provider.edn")
        );
    }
}

#[test]
fn exact_keyword_spec_requires_the_core_predicate() {
    let source = r#"
(ns demo.shadow-keyword
  (:refer-clojure :exclude [keyword?])
  (:require [clojure.spec.alpha :as s]))
(defn keyword? {:t/sig '[any? :-> boolean?]} [x] (string? x))
(s/def ::local keyword?)
(s/def ::core clojure.core/keyword?)

(defn local-result {:t/sig '[:-> any?]} []
  (s/conform ::local "opaque predicate"))
(defn core-result {:t/sig '[:-> #{:region}]} []
  (s/conform ::core :region))
(defn core-marker {:t/sig '[:-> #{false}]} []
  (s/valid? ::core ::s/invalid))
"#;
    let report = Session::default().check("shadow_keyword.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .all(|finding| finding.code == "PNX9001")
    );
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.message.contains("supported conversion"))
        .collect();
    let [finding] = findings.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9001", "{finding:#?}");
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "::local"
    );
}

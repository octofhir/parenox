use parenox_analysis::{Report, Session, Source};

const SPECS: &str = r#"
(ns demo.name-conformer (:require [clojure.spec.alpha :as s]))
(s/def ::code (s/conformer name))
(s/def ::code-alias ::code)
(s/def ::qualified-code (s/conformer clojure.core/name))
"#;

fn check(body: &str) -> Report {
    Session::default().check("name_conformer.clj", &format!("{SPECS}{body}"))
}

#[test]
fn direct_apply_callback_and_conformer_keep_exact_names() {
    let report = check(
        r#"
(defn strings {:t/sig '[:-> (s/tuple #{""} #{"topic/a/b"})]} []
  [(s/conform ::code "") (s/conform ::code-alias "topic/a/b")])
(defn keywords {:t/sig '[:-> (s/tuple #{"/"} #{"a/b"} #{"invalid"})]} []
  [(s/conform ::code :/)
   (s/conform ::qualified-code :topic/a/b)
   (s/conform ::code-alias ::s/invalid)])
(defn reserved-is-valid {:t/sig '[:-> #{true}]} [] (s/valid? ::code ::s/invalid))
(defn direct {:t/sig '[:-> #{"a/b"}]} []
  (try (name :topic/a/b) (catch Throwable _ :unexpected)))
(defn applied {:t/sig '[:-> #{"topic/a/b"}]} []
  (try (apply clojure.core/name ["topic/a/b"]) (catch Throwable _ :unexpected)))
(defn callback {:t/sig '[:-> (s/coll-of #{"a/b"} :kind vector?)]} []
  (mapv name [:topic/a/b]))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn an_optional_diagnostic_code_is_converted_only_when_present() {
    let report = check(
        r#"
(s/def ::message string?)
(s/def ::diagnostic (s/keys :req-un [::message] :opt-un [::code]))
(defn converted {:t/sig '[:-> {:message #{"problem"} :code #{"a/b"}}]} []
  (s/conform ::diagnostic {:message "problem" :code :topic/a/b}))
(defn reserved-field {:t/sig '[:-> #{"invalid"}]} []
  (:code (s/conform ::diagnostic {:message "problem" :code ::s/invalid})))
(defn omitted {:t/sig '[:-> {:message #{"problem"}}]} []
  (try (s/conform ::diagnostic {:message "problem"})
       (catch Throwable _ :unexpected)))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn known_non_named_values_have_exact_exception_effects() {
    let report = check(
        r#"
(defn nil-input {:t/sig '[:-> #{:nil-input}]} []
  (try (s/conform ::code nil)
       (catch ClassCastException _ :unexpected)
       (catch NullPointerException _ :nil-input)))
(defn boolean-input {:t/sig '[boolean? :-> #{:wrong-class}]} [value]
  (try (s/valid? ::code value)
       (catch NullPointerException _ :unexpected)
       (catch ClassCastException _ :wrong-class)))
(defn character-input {:t/sig '[char? :-> #{:wrong-class}]} [value]
  (try (name value)
       (catch NullPointerException _ :unexpected)
       (catch ClassCastException _ :wrong-class)))
(defn integer-input {:t/sig '[int? :-> #{:wrong-class}]} [value]
  (try (apply name [value])
       (catch NullPointerException _ :unexpected)
       (catch ClassCastException _ :wrong-class)))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn imprecise_names_can_be_nil_and_numeric_or_open_inputs_can_throw() {
    let body = r#"
(defn symbol-name {:t/sig '[symbol? :-> (s/nilable string?)]} [value]
  (try (s/conform ::code value) (catch Throwable _ :unexpected)))
(defn symbol-valid {:t/sig '[symbol? :-> #{true}]} [value]
  (s/valid? ::code value))
(defn numeric-name {:t/sig '[number? :-> (s/or :name (s/nilable string?) :error #{:name-error})]} [value]
  (try (s/conform ::code value) (catch Throwable _ :name-error)))
(defn arbitrary-int {:t/sig '[:-> #{true :name-error}]} []
  (try (s/valid? ::code 1N) (catch Throwable _ :name-error)))
(defn open-value {:t/sig '[any? :-> #{true :name-error}]} [value]
  (try (s/valid? ::code value) (catch Throwable _ :name-error)))
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{body}
(defn denies-nil {{:t/sig '[symbol? :-> string?]}} [value]
  (s/conform ::code value))
(defn denies-throw {{:t/sig '[number? :-> #{{true}}]}} [value]
  (try (s/valid? ::code value) (catch Throwable _ :name-error)))
(defn denies-success {{:t/sig '[:-> #{{:name-error}}]}} []
  (try (s/valid? ::code 1N) (catch Throwable _ :name-error)))
"#,
    ));
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 3, "{report:#?}");
    for finding in report.diagnostics {
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    }
}

#[test]
fn named_conformance_requires_loading_the_registration_namespace() {
    for mode in [":as", ":as-alias"] {
        let source = format!(
            r#"(ns consumer (:require [clojure.spec.alpha :as s] [demo.name-conformer {mode} wire]))
(defn converted {{:t/sig '[:-> #{{"a/b"}}]}} [] (s/conform ::wire/code-alias :topic/a/b))
(defn valid {{:t/sig '[:-> #{{true}}]}} [] (s/valid? ::wire/code-alias ::s/invalid))"#
        );
        let report = Session::default().check_project(
            &[
                Source {
                    path: "consumer.clj",
                    source: &source,
                },
                Source {
                    path: "demo/name_conformer.clj",
                    source: SPECS,
                },
            ],
            None,
        );
        if mode == ":as" {
            assert!(report.ok && report.complete, "{report:#?}");
        } else {
            let findings = &report.files[0].diagnostics;
            assert_eq!(findings.len(), 2, "{report:#?}");
            for finding in findings {
                assert_eq!(finding.code, "PNX9001", "{finding:#?}");
                assert!(finding.message.contains("registrations are loaded"));
            }
        }
    }
}

#[test]
fn unknown_input_projections_report_gaps_at_each_contract_use() {
    let report = check(
        r#"
(s/def ::row (s/keys :req-un [::code-alias]))
(defn inline {:t/sig '[(s/keys :req-un [::code]) :-> keyword?]} [_] :ok)
(defn projected {:t/sig '[::row :-> keyword?]} [_] :ok)
(s/fdef via-fdef :args (s/cat :code ::code-alias) :ret keyword?)
(defn via-fdef [_] :ok)
"#,
    );
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 3, "{report:#?}");
    for finding in report.diagnostics {
        assert_eq!(finding.code, "PNX9001", "{finding:#?}");
        assert!(
            finding.message.contains("unchecked input type"),
            "{finding:#?}"
        );
    }
}

#[test]
fn changing_the_conversion_refreshes_contract_findings_with_the_same_type() {
    let consumer = Source {
        path: "consumer.clj",
        source: r#"(ns consumer (:require [demo.name-conformer :as wire]))
(defn consume {:t/sig '[::wire/code-alias :-> keyword?]} [_] :ok)"#,
    };
    let opaque = SPECS.replace("(s/conformer name)", "(s/conformer identity)");
    let provider = Source {
        path: "demo/name_conformer.clj",
        source: &opaque,
    };
    let supported = Source {
        source: SPECS,
        ..provider
    };
    let mut session = Session::default();
    let before = session.check_project(&[consumer, provider], None);
    assert!(before.files[0].diagnostics.is_empty(), "{before:#?}");
    let after = session.check_project(&[consumer, supported], None);
    let [finding] = after.files[0].diagnostics.as_slice() else {
        panic!("{after:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
    assert!(
        finding.message.contains("unchecked input type"),
        "{finding:#?}"
    );
    let restored = session.check_project(&[consumer, provider], None);
    assert!(restored.files[0].diagnostics.is_empty(), "{restored:#?}");
}

#[test]
fn an_explicit_supported_domain_exports_without_the_spec_registry() {
    let mut session = Session::default();
    let unchecked = format!(
        r#"{SPECS}
(defn publish {{:t/sig '[::code :-> keyword?]}} [_] :sent)"#
    );
    let provider = Source {
        path: "demo/name_conformer.clj",
        source: &unchecked,
    };
    let export = session.export_externs(&[provider], &[]);
    assert!(
        !export.report.complete && export.edn.is_none(),
        "{export:#?}"
    );

    let checked = format!(
        r#"{SPECS}
(defn normalize {{:t/sig '[(s/or :text string? :key keyword?) :-> string?]}} [value]
  (s/conform ::code value))"#
    );
    let export = session.export_externs(
        &[Source {
            source: &checked,
            ..provider
        }],
        &[],
    );
    let edn = export.edn.unwrap_or_else(|| panic!("{:#?}", export.report));
    let externs = Source {
        path: "wire.edn",
        source: &edn,
    };
    let check_input = |input| {
        let source = format!(
            r#"(ns consumer (:require [demo.name-conformer :as wire]))
(defn normalize {{:t/sig '[:-> string?]}} [] (wire/normalize {input}))"#
        );
        Session::default().check_project_with_externs(
            &[Source {
                path: "consumer.clj",
                source: &source,
            }],
            &[externs],
        )
    };
    for input in [r#""topic/a/b""#, ":topic/a/b"] {
        let report = check_input(input);
        assert!(report.ok && report.complete, "{report:#?}");
    }
    let report = check_input("42");
    let [finding] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert_eq!(
        finding.contract.as_ref().unwrap().path.as_deref(),
        Some("wire.edn")
    );
}

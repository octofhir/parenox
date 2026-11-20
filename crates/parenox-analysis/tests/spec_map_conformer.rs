use parenox_analysis::{Report, Session, Source};

const SPECS: &str = r#"
(ns demo.code-lens (:require [clojure.spec.alpha :as s]))
(s/def :server-capabilities/code-lens-provider
  (s/conformer (fn [element] {:resolve-provider element})))
(s/def ::code-lens :server-capabilities/code-lens-provider)
"#;

fn check(body: &str) -> Report {
    Session::default().check("demo/code_lens.clj", &format!("{SPECS}{body}"))
}

#[test]
fn wrapper_conforms_every_value_and_keeps_the_original_field_type() {
    for (input, expected) in [
        ("nil", "nil?"),
        ("false", "#{false}"),
        ("::s/invalid", "#{::s/invalid}"),
        (":topic/item", "#{:topic/item}"),
        ("{:detail false}", "{:detail #{false}}"),
    ] {
        let report = check(&format!(
            r#"
(defn conform {{:t/sig '[:-> {{:resolve-provider {expected}}}]}} []
  (try (s/conform ::code-lens {input}) (catch Throwable _ :unexpected)))
(defn valid {{:t/sig '[:-> #{{true}}]}} []
  (try (s/valid? :server-capabilities/code-lens-provider {input})
       (catch Throwable _ :unexpected)))
"#
        ));
        assert!(report.ok && report.complete, "{input}: {report:#?}");
    }
}

#[test]
fn wrapper_preserves_rigid_parameters_and_has_a_total_input_projection() {
    let report = check(
        r#"
(defn poly {:t/sig '(t/all [a] [a :-> {:resolve-provider a}])} [x]
  (s/conform ::code-lens x))
(defn bounded {:t/sig '(t/all [(a :< keyword?)] [a :-> {:resolve-provider a}])} [x]
  (s/conform ::code-lens x))
(defn projected {:t/sig '[::code-lens :-> {:resolve-provider t/any}]} [x]
  (s/conform ::code-lens x))
(defn full-domain {:t/sig '[keyword? :-> {:resolve-provider keyword?}]} [x]
  (projected ::s/invalid)
  (poly x))
(defn derived {:t/sig '[:-> #{false}]} [] (:resolve-provider (poly false)))
(defn unreachable {:t/sig '[t/any :-> string?]} [x]
  (if (s/valid? ::code-lens x) "always valid" :unexpected))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(
        r#"
(defn loses-input {:t/sig '(t/all [a] [a :-> {:resolve-provider a}])} [_]
  (s/conform ::code-lens :other))
"#,
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert!(finding.message.contains(":other"), "{finding:#?}");
}

#[test]
fn a_source_less_consumer_keeps_the_generic_result_and_total_input_in_annotated_exports() {
    let provider = format!(
        r#"{SPECS}
(defn wrap {{:t/sig '(t/all [a] [a :-> {{:resolve-provider a}}])}} [input]
  (s/conform ::code-lens input))
(defn domain {{:t/sig '[::code-lens :-> keyword?]}} [_] :accepted)
"#
    );
    let provider = Source {
        path: "demo/code_lens.clj",
        source: &provider,
    };
    let mut session = Session::default();
    let export = session.export_externs(&[provider], &[]);
    let edn = export.edn.unwrap_or_else(|| panic!("{:#?}", export.report));
    let externs = Source {
        path: "provider.edn",
        source: &edn,
    };
    let validated = session.validate_externs(&[provider], &[externs]);
    assert!(validated.ok && validated.complete, "{validated:#?}");
    let source = r#"
(ns consumer (:require [demo.code-lens :as lens] [clojure.spec.alpha :as s]))
(defn marker {:t/sig '[:-> #{::s/invalid}]} []
  (lens/domain ::s/invalid)
  (:resolve-provider (lens/wrap ::s/invalid)))
"#;
    let check = |source| {
        Session::default().check_project_with_externs(
            &[Source {
                path: "consumer.clj",
                source,
            }],
            &[externs],
        )
    };
    let report = check(source);
    assert!(report.ok && report.complete, "{report:#?}");
    let changed = source.replace("(lens/wrap ::s/invalid)", "(lens/wrap false)");
    let report = check(&changed);
    let [finding] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("false"));
}

#[test]
fn unsupported_bodies_and_unloaded_definitions_report_gaps() {
    let report = Session::default().check(
        "unchecked.clj",
        &format!(
            "{}\n(defn wrap [] (s/conform ::code-lens false))",
            SPECS.replace(
                "{:resolve-provider element}",
                "{:resolve-provider (identity element)}"
            )
        ),
    );
    assert!(!report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX9001")
    );

    let source = r#"
(ns consumer (:require [demo.code-lens :as-alias lens] [clojure.spec.alpha :as s]))
(defn wrap {:t/sig '[:-> {:resolve-provider #{false}}]} []
  (s/conform ::lens/code-lens false))
"#;
    let report = Session::default().check_project(
        &[
            Source {
                path: "consumer.clj",
                source,
            },
            Source {
                path: "demo/code_lens.clj",
                source: SPECS,
            },
        ],
        None,
    );
    assert!(!report.complete, "{report:#?}");
    assert!(
        report.files[0].diagnostics.iter().any(|finding| {
            finding.code == "PNX9001" && finding.message.contains("registrations are loaded")
        }),
        "{report:#?}"
    );
}

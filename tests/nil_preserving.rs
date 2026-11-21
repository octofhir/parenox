use parenox::{Session, Source};

const CONTRACT: &str = "{vendor.uri/decode (t/nil-preserving [string? :-> string?])
vendor.uri/format-option (t/nil-preserving [boolean? :-> string?])
vendor.uri/through (t/all [a r] [(clojure.spec.alpha/fspec :args (clojure.spec.alpha/cat :value a) :ret r) a :-> r])}";

#[test]
fn nil_preserving_externs_specialize_without_copying_literal_input_values() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s] [vendor.uri :as uri]))
(defn text {:t/sig '[:-> string?]} [] (uri/decode "a%20b"))
(defn nothing {:t/sig '[:-> nil?]} [] (uri/decode nil))
(defn false-is-a-value {:t/sig '[:-> string?]} [] (uri/format-option false))
(defn mixed {:t/sig '[(s/nilable string?) :-> (s/nilable string?)]} [input] (uri/decode input))
(defn via-spread {:t/sig '[:-> nil?]} [] (apply uri/decode [nil]))
(defn accepts-decoder {:t/sig '[(s/fspec :args (s/cat :text string?) :ret string?) :-> string?]}
  [f] (f "a%20b"))
(defn as-callback {:t/sig '[:-> string?]} [] (accepts-decoder uri/decode))
(defn through-text {:t/sig '[:-> string?]} [] (uri/through uri/decode "a%20b"))
(defn through-nil {:t/sig '[:-> nil?]} [] (uri/through uri/decode nil))"#;
    let mut session = Session::default();
    let externs = [Source {
        path: "uri.edn",
        source: CONTRACT,
    }];
    let check = |session: &mut Session, source| {
        session.check_project_with_externs(
            &[Source {
                path: "demo.clj",
                source,
            }],
            &externs,
        )
    };
    let report = check(&mut session, source);
    assert!(report.ok && report.complete, "{report:#?}");

    let bad = format!(
        "{source}\n(defn wrong {{:t/sig '[:-> string?]}} [] (subs (uri/decode false) 0))\n\
         (defn literal {{:t/sig '[:-> #{{\"a%20b\"}}]}} [] (uri/decode \"a%20b\"))"
    );
    let report = check(&mut session, &bad);
    let [argument, result] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(argument.code, "PNX2322");
    assert_eq!(
        &bad[argument.span.start.offset..argument.span.end.offset],
        "false"
    );
    assert_eq!(argument.expected.as_deref(), Some("Nil | String"));
    assert_eq!(result.code, "PNX2322");
    assert_eq!(result.actual.as_deref(), Some("String"));
}

#[test]
fn nil_preservation_requires_a_single_non_nil_input_and_result() {
    let mut session = Session::default();
    for signature in [
        "[string? string? :-> string?]",
        "[string? & string? :-> string?]",
        "[nil? :-> string?]",
        "[string? :-> (clojure.spec.alpha/nilable string?)]",
    ] {
        let externs = format!("{{vendor.uri/decode (t/nil-preserving {signature})}}");
        let report = session.check_project_with_externs(
            &[],
            &[Source {
                path: "uri.edn",
                source: &externs,
            }],
        );
        let [finding] = report.files[0].diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, "PNX9001");
        assert!(
            finding.message.contains("both excluding Nil"),
            "{finding:#?}"
        );
    }
}

#[test]
fn replacing_the_correlation_rechecks_an_unchanged_consumer() {
    let consumer = [Source {
        path: "demo.clj",
        source: r#"(ns demo (:require [clojure.spec.alpha :as s] [vendor.uri :as uri]))
(defn fragment {:t/sig '[(s/nilable string?) :-> (s/nilable string?)]} [text]
  (when (some? text) (subs (uri/decode text) 0)))"#,
    }];
    let check = |session: &mut Session, contract| {
        session.check_project_with_externs(
            &consumer,
            &[Source {
                path: "uri.edn",
                source: contract,
            }],
        )
    };
    let flat = "{vendor.uri/decode [(clojure.spec.alpha/nilable string?) :-> (clojure.spec.alpha/nilable string?)]}";
    let mut session = Session::default();
    assert!(check(&mut session, CONTRACT).ok);
    let report = check(&mut session, flat);
    let [finding] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("Nil | String"));
    assert!(check(&mut session, CONTRACT).ok);
}

#[test]
fn an_unchecked_argument_does_not_prove_the_result_of_a_correlated_extern() {
    let source = Source {
        path: "demo.clj",
        source: r#"(ns demo (:require [vendor.uri :as uri]))
(defn normalize [input]
  (when-not (nil? input) (uri/encode (uri/decode input) :fragment)))"#,
    };
    let externs = Source {
        path: "uri.edn",
        source: "{vendor.uri/decode (t/nil-preserving [string? :-> string?])
                  vendor.uri/encode [string? keyword? :-> string?]}",
    };
    let report = Session::default().check_project_with_externs(&[source], &[externs]);
    assert!(!report.complete, "{report:#?}");
    let findings = &report.files[0].diagnostics;
    assert!(
        findings.iter().any(|finding| finding.code == "PNX9002"),
        "{report:#?}"
    );
    assert!(
        findings.iter().all(|finding| finding.severity == "warning"),
        "{report:#?}"
    );
}

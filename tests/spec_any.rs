use parenox::{Report, Session, Source};

fn check(body: &str) -> Report {
    let (specs, _) = include_str!("fixtures/spec-any.clj")
        .split_once(";; JVM-only")
        .unwrap();
    Session::default().check("any_spec.clj", &format!("{specs}\n{body}"))
}

#[test]
fn any_spec_preserves_falsey_values_and_rejects_only_the_invalid_sentinel() {
    let body = r#"
(s/def ::alias ::value)
(s/fdef accept :args (s/cat :value ::alias) :ret keyword?)
(defn accept [value] :accepted)
(defn send {:t/sig '[:-> keyword?]} []
  (accept nil)
  (accept false)
  (accept [::s/invalid])
  (accept :ok))
(defn valid-falsey {:t/sig '[:-> #{true}]} [] (s/valid? ::alias nil))
(defn reject-invalid {:t/sig '[:-> #{false}]} [] (s/valid? ::alias ::s/invalid))
(defn conform-falsey {:t/sig '[:-> #{false}]} [] (s/conform ::alias false))
(defn plain-predicate {:t/sig '[:-> #{true}]} [] (any? ::s/invalid))
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");
    let report = check(&body.replace("(accept :ok)", "(accept ::s/invalid)"));
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(finding.code, "PNX2322");
    assert!(finding.message.contains("Spec value"), "{finding:#?}");
}

#[test]
fn any_spec_is_known_but_unknown_values_are_unproven() {
    for input in ["keyword?", "a"] {
        let parameters = if input == "a" {
            "(t/all [a] [a :-> boolean?])"
        } else {
            "[keyword? :-> boolean?]"
        };
        let report = check(&format!(
            "(defn accept {{:t/sig '[::value :-> boolean?]}} [x] true)\n
             (defn forward {{:t/sig '{parameters}}} [x] (accept x))"
        ));
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}")
        };
        assert!(!report.complete);
        assert_eq!(finding.code, "PNX9004", "{report:#?}");
    }

    let report = check(
        r#"(defn require-truthy {:t/sig '[::value :-> identity]} [x]
              (if x x :fallback))
           (defn require-falsey {:t/sig '[::value :-> #{nil false}]} [x]
              (if x false x))
           (defn nil-test {:t/sig '[::value :-> #{nil}]} [x]
              (if (nil? x) x nil))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn explicitly_kinded_collections_check_members_and_map_entries_correctly() {
    let source = r#"
(defn vector-values {:t/sig '[::values :-> ::values]} [x] x)
(defn map-entries {:t/sig '[::entries :-> ::entries]} [x] x)
(defn send {:t/sig '[:-> ::entries]} []
  (vector-values [nil false [::s/invalid]])
  (map-entries {::s/invalid ::s/invalid}))
"#;
    let report = check(source);
    assert!(report.ok && report.complete, "{report:#?}");
    for bad in ["[::s/invalid]", "'(nil false)", "nil"] {
        let report = check(&source.replace("[nil false [::s/invalid]]", bad));
        assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
        assert_eq!(report.diagnostics[0].code, "PNX2322", "{report:#?}");
    }

    let report = check(
        r#"
(s/def ::strings (s/coll-of string?))
(defn strings {:t/sig '[:-> (s/tuple #{"ready"})]} []
  (s/conform ::strings ["ready"]))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn unrestricted_collection_checks_shallow_elements_in_all_modeled_collections() {
    let source = r#"
(s/def ::alias (s/coll-of ::value :gen-max 4))
(s/fdef roundtrip :args (s/cat :input ::alias) :ret ::collection)
(defn roundtrip [input] input)
(defn send {:t/sig '[:-> ::collection]} []
  (roundtrip [])
  (roundtrip '(nil false))
  (roundtrip #{nil false})
  (roundtrip {::s/invalid ::s/invalid})
  (roundtrip (first {:ok nil}))
  (roundtrip [[::s/invalid]]))
"#;
    let report = check(source);
    assert!(report.ok && report.complete, "{report:#?}");
    for input in [
        "nil",
        "\"text\"",
        "[::s/invalid]",
        "#{::s/invalid}",
        "'(:clojure.spec.alpha/invalid)",
        "(first {::s/invalid ::s/invalid})",
    ] {
        let report = check(&source.replace("[[::s/invalid]]", input));
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, "PNX2322");
        if input == "[::s/invalid]" {
            assert!(finding.message.contains("item 0"), "{finding:#?}");
            assert_eq!(finding.expected.as_deref(), Some("Spec value"));
        } else {
            assert!(
                finding.message.contains("Collection of spec values"),
                "{finding:#?}"
            );
        }
    }
}

#[test]
fn collection_validation_and_conformance_use_collection_evidence() {
    let report = check(
        r#"
(defn invalid-member {:t/sig '[:-> #{false}]} []
  (s/valid? ::collection [::s/invalid]))
(defn invalid-string {:t/sig '[:-> #{false}]} []
  (s/valid? ::collection "abc"))
(defn valid-map {:t/sig '[:-> #{true}]} []
  (s/valid? ::collection {::s/invalid ::s/invalid}))
(defn conform-vector {:t/sig '[:-> (s/coll-of string? :kind vector?)]} []
  (s/conform ::collection ["good"]))
(defn validated-first {:t/sig '[any? :-> any?]} [input]
  (if (s/valid? ::collection input) (first input) nil))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");

    let uncertain = r#"(defn maybe-elements {:t/sig '[(s/coll-of keyword? :kind vector?) :-> #{false}]} [x]
                          (s/valid? ::collection x))"#;
    let report = check(uncertain);
    assert_eq!(report.diagnostics[0].code, "PNX2322", "{report:#?}");

    let report = check(
        r#"(s/def ::tagged (s/coll-of (s/or :value any?)))
           (defn conform-tagged {:t/sig '[:-> ::collection]} []
             (s/conform ::tagged [nil]))"#,
    );
    assert!(!report.complete, "{report:#?}");
    assert!(
        report.diagnostics.iter().any(|finding| {
            finding
                .message
                .contains("s/conform requires a named spec with a supported conversion")
        }),
        "{report:#?}"
    );
}

#[test]
fn a_shadowed_any_predicate_remains_unchecked() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
(defn any? {:t/sig '[string? :-> boolean?]} [x] false)
(s/def ::local any?)
(s/def ::qualified clojure.core/any?)
(defn qualified {:t/sig '[:-> #{true}]} [] (s/valid? ::qualified false))
"#;
    let report = Session::default().check("demo.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert!(!report.complete);
    assert!(
        finding
            .message
            .contains("Source predicate any? is unchecked")
    );
}

#[test]
fn exports_an_any_contract_and_retains_the_invalid_sentinel_check() {
    let source = Source {
        path: "provider.clj",
        source: r#"(ns provider (:require [clojure.spec.alpha :as s]))
                   (defn accept {:t/sig '[any? :-> string?]} [x] "ok")
                   (defn collection {:t/sig '[(s/coll-of any?) :-> string?]} [x] "ok")"#,
    };
    let export = Session::default().export_externs(&[source], &[]);
    let edn = export.edn.unwrap_or_else(|| panic!("{:#?}", export.report));
    let externs = [Source {
        path: "provider.edn",
        source: &edn,
    }];
    let client = r#"(ns client (:require [provider :as p] [clojure.spec.alpha :as s]))
(defn call {:t/sig '[:-> string?]} []
  (p/collection {::s/invalid ::s/invalid})
  (p/accept nil))"#;
    let report = |source: &str| {
        Session::default().check_project_with_externs(
            &[Source {
                path: "client.clj",
                source,
            }],
            &externs,
        )
    };
    let good = report(client);
    assert!(good.ok && good.complete, "{good:#?}");
    let bad = report(&client.replace("(p/accept nil)", "(p/accept :clojure.spec.alpha/invalid)"));
    assert_eq!(bad.files[0].diagnostics[0].code, "PNX2322", "{bad:#?}");
    let bad = report(&client.replace("{::s/invalid ::s/invalid}", "[::s/invalid]"));
    assert_eq!(bad.files[0].diagnostics[0].code, "PNX2322", "{bad:#?}");
}

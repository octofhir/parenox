use parenox::{ProjectReport, Session, Source};

const LIBRARY: Source<'_> = Source {
    path: "wire.clj",
    source: r#"
(ns wire (:require [clojure.spec.alpha :as s]))
(s/def ::name string?)
(s/def ::entry (s/keys :req-un [::name]))
(s/def ::entries (s/coll-of ::entry))
"#,
};
const ROOT: Source<'_> = Source {
    path: "entries.edn",
    source: "{:wire/entries {:spec (clojure.spec.alpha/coll-of :wire/entry) :ns wire}}",
};
const ENVELOPE: Source<'_> = Source {
    path: "envelope.edn",
    source: "{:wire/envelope {:spec (clojure.spec.alpha/keys :req-un [:wire/entries]) :ns wire}}",
};
const DICTIONARY: Source<'_> = Source {
    path: "by-uri.edn",
    source: r#"{:wire/by-uri {:spec (clojure.spec.alpha/coll-of
      (clojure.spec.alpha/tuple clojure.core/string? :wire/entries) :kind clojure.core/map?) :ns wire}}"#,
};
const ELEMENT: Source<'_> = Source {
    path: "entry.edn",
    source: r#"
{:wire/entry {:spec (clojure.spec.alpha/keys :req-un [:wire/name]) :ns wire}
 :wire/name {:spec clojure.core/string? :ns wire}}
"#,
};

fn check(source: &str, externs: &[Source<'_>]) -> ProjectReport {
    let source =
        format!("(ns client (:require [clojure.spec.alpha :as s] [wire :as w]))\n{source}");
    Session::default().check_project_with_externs(
        &[Source {
            path: "client.clj",
            source: &source,
        }],
        externs,
    )
}

#[test]
fn a_source_free_collection_checks_bound_and_quoted_vectors() {
    let checked = check(
        r#"
(defn bound {:t/sig '[:-> (s/tuple {:name #{"first"}} {:name #{"second"}})]} []
  (let [entries [{:name "first"} {:name "second"}]]
    (s/conform ::w/entries entries)))
(defn quoted {:t/sig '[:-> #{true}]} []
  (s/valid? ::w/entries '[{:name "quoted"}]))
(defn empty-valid {:t/sig '[:-> #{true}]} []
  (empty? (s/conform ::w/entries [])))
"#,
        &[ROOT, ELEMENT],
    );
    assert!(checked.ok && checked.complete, "{checked:#?}");
}

#[test]
fn a_proven_nested_vector_is_invalid_as_an_element_map() {
    let envelope = Source {
        path: "envelope.edn",
        source: r#"{:wire/envelope {:spec (clojure.spec.alpha/keys :req-un [:wire/entries]) :ns wire}}"#,
    };
    let checked = check(
        r#"(defn invalid {:t/sig '[:-> #{false}]} []
             (s/valid? ::w/envelope {:entries [[]]}))"#,
        &[ROOT, ELEMENT, envelope],
    );
    assert!(checked.ok && checked.complete, "{checked:#?}");
}

#[test]
fn a_nested_collection_keeps_proof_through_map_lookup_and_let() {
    let checked = check(
        r#"
(defn envelope {:t/sig '[:-> {:entries (s/tuple {:name #{"quoted"}})}]} []
  (let [holder '{:payload [{:name "quoted"}]}
        entries (:payload holder)]
    (s/conform ::w/envelope {:entries entries})))
"#,
        &[ENVELOPE, ROOT, ELEMENT],
    );
    assert!(checked.ok && checked.complete, "{checked:#?}");
}

#[test]
fn a_source_free_dictionary_checks_keys_before_values_and_accounts_for_unknown_entry_order() {
    let entry = Source {
        source: r#"
{:wire/entry {:spec (clojure.spec.alpha/keys :req-un [:wire/coordinate]) :ns wire}
 :wire/coordinate {:spec (clojure.spec.alpha/and clojure.core/integer?
                           (clojure.spec.alpha/conformer clojure.core/int)) :ns wire}}
"#,
        ..ELEMENT
    };
    let checked = check(
        r#"
(defn skip {:t/sig '[(s/tuple {:coordinate integer?}) :-> #{false}]} [xs]
  (try (s/valid? ::w/by-uri {:bad xs}) (catch ArithmeticException _ :unexpected)))
(defn reject {:t/sig '[:-> #{false}]} [] (s/valid? ::w/by-uri []))
(defn order {:t/sig '[:-> #{false :overflow}]} []
  (try (s/valid? ::w/by-uri {"file:///one" [{:coordinate 2147483648}] :bad []})
       (catch ArithmeticException _ :overflow)))
(defn eager {:t/sig '[:-> #{:caught}]} []
  (try (s/conform ::w/by-uri {:bad (throw (ex-info "eager" {}))})
       (catch clojure.lang.ExceptionInfo _ :caught)))
"#,
        &[DICTIONARY, ROOT, entry],
    );
    assert!(checked.ok && checked.complete, "{checked:#?}");

    let checked = check(
        r#"(defn order {:t/sig '[:-> #{false}]} []
              (try (s/valid? ::w/by-uri {"file:///one" [{:coordinate 2147483648}] :bad []})
                   (catch ArithmeticException _ :overflow)))"#,
        &[DICTIONARY, ROOT, entry],
    );
    assert!(
        checked.files[0]
            .diagnostics
            .iter()
            .any(|finding| { finding.code == "PNX2322" && finding.message.contains(":overflow") }),
        "{checked:#?}"
    );
}

#[test]
fn a_dictionary_consumer_is_rechecked_when_a_separate_transitive_extern_changes() {
    let source = Source {
        path: "client.clj",
        source: r#"(ns client (:require [clojure.spec.alpha :as s] [wire :as w]))
        (defn validate {:t/sig '[:-> #{true}]} []
          (s/valid? ::w/by-uri {"file:///doc" [{:name "old"}]}))"#,
    };
    let updated = ELEMENT.source.replace("clojure.core/string?", "#{\"new\"}");
    let mut session = Session::default();
    let accepted = session.check_project_with_externs(&[source], &[DICTIONARY, ROOT, ELEMENT]);
    assert!(accepted.ok && accepted.complete, "{accepted:#?}");
    let changed = session.check_project_with_externs(
        &[source],
        &[
            DICTIONARY,
            ROOT,
            Source {
                source: &updated,
                ..ELEMENT
            },
        ],
    );
    assert!(
        changed
            .files
            .iter()
            .any(|file| file.diagnostics.iter().any(|finding| {
                finding.code == "PNX2322" && finding.message.contains("validate")
            })),
        "{changed:#?}"
    );
    let restored = session.check_project_with_externs(&[source], &[DICTIONARY, ROOT, ELEMENT]);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn a_nested_collection_loses_proof_when_a_branch_uses_an_opaque_vector() {
    let checked = check(
        r#"
(defn envelope {:t/sig '[boolean? (s/tuple {:name #{"quoted"}}) :-> boolean?]} [choose? opaque]
  (let [holder '{:payload [{:name "quoted"}]}
        entries (if choose? (:payload holder) opaque)]
    (s/valid? ::w/envelope {:entries entries})))
"#,
        &[ENVELOPE, ROOT, ELEMENT],
    );
    let [finding] = checked.files[0].diagnostics.as_slice() else {
        panic!("{checked:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
    assert!(
        finding.message.contains("bounded literal vector"),
        "{finding:#?}"
    );
    assert!(!checked.complete);
}

#[test]
fn a_source_free_collection_leaves_unproven_shapes_unchecked() {
    let oversized = format!("[{}]", r#"{:name "item"} "#.repeat(33));
    for (signature, parameters, input) in [
        ("[(s/tuple {:name string?}) :-> boolean?]", "[xs]", "xs"),
        ("[:-> boolean?]", "[]", r#"#{{:name "set"}}"#),
        ("[:-> boolean?]", "[]", oversized.as_str()),
    ] {
        let checked = check(
            &format!(
                "(defn probe {{:t/sig '{signature}}} {parameters} (s/valid? ::w/entries {input}))"
            ),
            &[ROOT, ELEMENT],
        );
        let [finding] = checked.files[0].diagnostics.as_slice() else {
            panic!("{checked:#?}");
        };
        assert_eq!(finding.code, "PNX9001");
        assert!(finding.message.contains("bounded literal vector"));
        assert!(!checked.complete);
    }
    let checked = check(
        r#"(defn probe {:t/sig '[:-> t/any]} []
             (s/conform ::w/entries #{{:name "set"}}))"#,
        &[ROOT, ELEMENT],
    );
    assert!(
        checked.files[0].diagnostics.iter().any(|finding| {
            finding.code == "PNX9001" && finding.message.contains("bounded literal vector")
        }),
        "{checked:#?}"
    );
}

#[test]
fn a_qualified_extension_cannot_use_an_opaque_source_collection() {
    let wire = format!(
        "{}\n(s/def ::outer (s/keys :req-un [::name]))",
        LIBRARY.source
    );
    let client = Source {
        path: "client.clj",
        source: r#"(ns client (:require [clojure.spec.alpha :as s] [wire :as w]))
(defn api {:t/sig '[(s/tuple {:name string?}) :-> t/any]} [xs]
  (s/conform ::w/outer {:name "outer" :wire/entries xs}))"#,
    };
    let checked = Session::default().check_project(
        &[
            Source {
                source: &wire,
                ..LIBRARY
            },
            client,
        ],
        None,
    );
    assert!(
        checked.files[1].diagnostics.iter().any(|finding| {
            finding.code == "PNX9001" && finding.message.contains("undeclared qualified key")
        }),
        "{checked:#?}"
    );
}

#[test]
fn a_qualified_dictionary_field_keeps_source_conformance() {
    let wire = format!(
        "{}\n(s/def ::outer (s/keys :req-un [::name]))\n\
         (s/def ::by-uri (s/coll-of (s/tuple string? ::entries) :kind map?))",
        LIBRARY.source
    );
    let client = Source {
        path: "client.clj",
        source: r#"(ns client (:require [clojure.spec.alpha :as s] [wire :as w]))
(defn name-from-extension {:t/sig '[:-> #{"entry"}]} []
  (get-in (s/conform ::w/outer {:name "outer" ::w/by-uri {"file:///one" [{:name "entry"}]}})
          [::w/by-uri "file:///one" 0 :name]))"#,
    };
    let checked = Session::default().check_project(
        &[
            Source {
                source: &wire,
                ..LIBRARY
            },
            client,
        ],
        None,
    );
    assert!(checked.ok && checked.complete, "{checked:#?}");
}

#[test]
fn collection_arguments_are_evaluated_before_the_first_element_is_conformed() {
    let checked = check(
        r#"
(defn eager-vector {:t/sig '[:-> #{:caught}]} []
  (try (s/conform ::w/entries [nil (throw (ex-info "eager" {}))])
       (catch clojure.lang.ExceptionInfo _ :caught)))
(defn eager-valid-extra {:t/sig '[:-> #{:caught}]} []
  (try (s/valid? ::w/entries [nil] (throw (ex-info "eager" {})))
       (catch clojure.lang.ExceptionInfo _ :caught)))
"#,
        &[ROOT, ELEMENT],
    );
    assert!(checked.ok && checked.complete, "{checked:#?}");
}

#[test]
fn a_scalar_extern_validity_check_does_not_bound_an_arbitrary_input_integer() {
    let coordinate = Source {
        path: "coordinate.edn",
        source: r#"{:wire/coordinate
          {:spec (clojure.spec.alpha/and clojure.core/integer?
                   (clojure.spec.alpha/conformer clojure.core/int)) :ns wire}}"#,
    };
    let checked = check(
        r#"(defn keep-original {:t/sig '[integer? :-> ::w/coordinate]} [x]
             (if (s/valid? ::w/coordinate x) x 0))"#,
        &[coordinate],
    );
    assert!(
        checked.files[0].diagnostics.iter().any(|finding| {
            matches!(finding.code.as_str(), "PNX2322" | "PNX9004")
                && finding.message.contains("keep-original")
        }),
        "{checked:#?}"
    );
}

#[test]
fn portable_collections_need_an_earlier_local_map_element_spec() {
    let mut session = Session::default();
    let selected = [":wire/entries".into()];
    let proposed = session
        .propose_contracts(&[LIBRARY], &[], &selected)
        .unwrap();
    assert!(proposed.externs.unwrap().contains(":wire/entries {:spec"));

    let early = LIBRARY
        .source
        .replace("(s/def ::entries (s/coll-of ::entry))\n", "")
        .replace(
            "(s/def ::entry ",
            "(s/def ::entries (s/coll-of ::entry))\n(s/def ::entry ",
        );
    let proposed = session
        .propose_contracts(
            &[Source {
                source: &early,
                ..LIBRARY
            }],
            &[],
            &selected,
        )
        .unwrap();
    assert_eq!(proposed.externs.as_deref(), Some("{}\n"));

    let foreign = ELEMENT.source.replace(":ns wire", ":ns foreign");
    for source in [
        foreign.as_str(),
        "{:wire/entry {:spec clojure.core/string? :ns wire}}",
    ] {
        let checked = check(
            "(defn validate {:t/sig '[:-> boolean?]} [] (s/valid? ::w/entries []))",
            &[ROOT, Source { source, ..ELEMENT }],
        );
        assert!(!checked.complete, "{checked:#?}");
        assert!(
            checked.files[1].diagnostics.iter().any(|finding| {
                finding.code == "PNX9001"
                    && finding
                        .message
                        .contains("supported map element spec :wire/entry")
            }),
            "{checked:#?}"
        );
    }
}

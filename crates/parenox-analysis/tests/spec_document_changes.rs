use parenox_analysis::{Report, Session, Source};

const PROJECTION: &str = r#"
(s/def ::overwrite any?)
(s/def ::ignore-if-exists any?)
(s/def ::document-changes
  (s/and (s/coll-of ::document-changes-entry) (s/conformer #(map second %))))
(s/def ::workspace-edit (s/keys :opt-un [::document-changes]))
(s/def ::workspaces (s/coll-of ::workspace-edit))
"#;

const INPUTS: &str = r#"
(def creation {:kind "create" :uri "file:///new"})
(def rename {:kind "rename" :old-uri "file:///old" :new-uri "file:///after"})
(def edit {:text-document {:uri "file:///old" :version 1N} :edits []})
(def overflow {:text-document {:uri "file:///old" :version 2147483648} :edits []})
"#;

fn specs() -> &'static str {
    include_str!("../../../tests/fixtures/spec-document-changes-entry.clj")
        .split_once(";; JVM-only")
        .unwrap()
        .0
}

fn check(body: &str) -> Report {
    Session::default().check(
        "document_changes.clj",
        &format!("{}{PROJECTION}{INPUTS}{body}", specs()),
    )
}

#[test]
fn literal_sets_project_untagged_members_without_assuming_order() {
    let report = check(
        r#"
(defn projected {:t/sig '[:-> (s/coll-of map? :kind seq?)]} []
  (s/conform ::document-changes #{creation rename}))
(defn first-kind {:t/sig '[:-> #{"create" "rename"}]} []
  (:kind (first (s/conform ::document-changes #{creation rename}))))
(defn singleton {:t/sig '[:-> (s/and int? #{1})]} []
  (get-in (first (s/conform ::document-changes #{edit})) [:text-document :version]))
(defn valid {:t/sig '[:-> #{true}]} []
  (s/valid? ::document-changes #{creation rename edit}))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn validation_of_a_set_with_an_open_map_keeps_broad_outcomes() {
    let report = check(
        r#"
(defn valid-set? {:t/sig '[map? :-> #{true false :error}]} [value]
  (try (s/valid? ::document-changes #{value})
       (catch Throwable _ :error)))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn set_conformance_and_validation_keep_both_possible_failure_modes() {
    let body = r#"
(defn invalid-set {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::document-changes #{nil}))
(defn invalid-set? {:t/sig '[:-> #{false}]} []
  (s/valid? ::document-changes #{nil}))
(defn overflow-set {:t/sig '[:-> #{:overflow}]} []
  (try (s/conform ::document-changes #{overflow})
       (catch ArithmeticException _ :overflow)))
(defn overflow-set? {:t/sig '[:-> #{:overflow}]} []
  (try (s/valid? ::document-changes #{overflow})
       (catch ArithmeticException _ :overflow)))
(defn mixed-set {:t/sig '[boolean? :-> #{::s/invalid :overflow}]} [overflow-first?]
  (let [uri (if overflow-first? "file:///overflow-0" "file:///overflow-3")]
    (try (s/conform ::document-changes
                    #{:invalid-document-change {:text-document {:uri uri :version 2147483648}
                                                :edits []}})
         (catch ArithmeticException _ :overflow))))
(defn mixed-set? {:t/sig '[boolean? :-> #{false :overflow}]} [overflow-first?]
  (let [uri (if overflow-first? "file:///overflow-0" "file:///overflow-3")]
    (try (s/valid? ::document-changes
                   #{:invalid-document-change {:text-document {:uri uri :version 2147483648}
                                               :edits []}})
         (catch ArithmeticException _ :overflow))))
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");

    for (conform, valid) in [
        ("#{::s/invalid}", "#{false}"),
        ("#{:overflow}", "#{:overflow}"),
    ] {
        let restricted = body
            .replace("#{::s/invalid :overflow}", conform)
            .replace("#{false :overflow}", valid);
        let report = check(&restricted);
        assert!(!report.ok && report.complete, "{report:#?}");
        assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
        for finding in report.diagnostics {
            assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        }
    }
}

#[test]
fn a_set_can_deduplicate_equal_maps_during_conformance() {
    let report = check(
        r#"
(def collapse-values {1 :same 2 :same})
(s/def :extra/collapse (s/and int? collapse-values (s/conformer collapse-values)))
(defn collapse {:t/sig '[:-> #{:same}]} []
  (let [result (s/conform ::document-changes
                          #{{:kind "create" :uri "file:///same" :extra/collapse 1}
                            {:kind "create" :uri "file:///same" :extra/collapse 2}})]
    (:extra/collapse (first result))))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn set_creation_can_throw_before_conformance_on_equal_computed_members() {
    let body = r#"
(defn maybe-duplicate {:t/sig '[boolean? :-> (s/or :success (s/coll-of map? :kind seq?)
                                                :duplicate #{:duplicate})]} [duplicate?]
  (let [other (if duplicate? creation rename)]
    (try (s/conform ::document-changes #{creation other})
         (catch IllegalArgumentException _ :duplicate))))
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");

    let restricted = body.replace("#{:duplicate}", "#{:different}");
    let report = check(&restricted);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert!(finding.actual.as_deref().unwrap().contains(":duplicate"));
}

#[test]
fn ordered_collections_project_to_an_exact_sequence_of_untagged_values() {
    let mut body = r#"
(s/def ::qualified
  (s/and (s/coll-of ::document-changes-entry)
         (s/conformer #(clojure.core/map clojure.core/second %1))))
(defn qualified {:t/sig '[:-> (s/coll-of ::create-file :kind seq?)]} []
  (s/conform ::qualified [creation]))
(defn converted-version {:t/sig '[:-> int?]} []
  (:version (:text-document (first (s/conform ::document-changes [edit])))))
"#
    .to_owned();
    for (label, input) in [
        ("vector", "[creation rename]"),
        ("list", "(list creation rename)"),
        ("seq", "(seq [creation rename])"),
        (
            "map-entry",
            "(clojure.lang.MapEntry/create creation rename)",
        ),
    ] {
        body.push_str(&format!(
            r#"
(defn {label}-sequence {{:t/sig '[:-> (s/coll-of map? :kind seq?)]}} []
  (s/conform ::document-changes {input}))
(defn {label}-valid {{:t/sig '[:-> #{{true}}]}} []
  (s/valid? ::document-changes {input}))
(defn {label}-order {{:t/sig '[:-> (s/tuple #{{"file:///new"}} #{{"file:///after"}} nil?)]}} []
  (let [result (s/conform ::document-changes {input})]
    [(:uri (first result)) (:new-uri (first (rest result))) (next (rest result))]))
"#
        ));
    }
    let report = check(&body);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn the_projection_is_a_sequence_without_a_vector_or_list_shape() {
    let report = check(
        r#"
(defn requires-vector {:t/sig '[:-> (s/coll-of map? :kind vector?)]} []
  (s/conform ::document-changes [creation]))
(defn requires-list {:t/sig '[:-> (s/coll-of map? :kind list?)]} []
  (s/conform ::document-changes (list creation)))
"#,
    );
    let [vector, list] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    for (finding, name) in [(vector, "requires-vector"), (list, "requires-list")] {
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().name,
            format!("demo.document-changes-entry/{name}")
        );
    }
}

#[test]
fn empty_collections_produce_an_empty_sequence_object_and_invalid_inputs_fail() {
    let mut body = String::new();
    for (label, input) in [
        ("vector", "[]"),
        ("list", "'()"),
        ("map", "{}"),
        ("set", "#{}"),
    ] {
        body.push_str(&format!(
            r#"
(defn {label}-empty
  {{:t/sig '[:-> (s/tuple (s/coll-of map? :kind seq?) nil? nil? #{{true}})]}} []
  (let [result (s/conform ::document-changes {input})]
    [result (seq result) (first result) (empty? result)]))
(defn {label}-valid {{:t/sig '[:-> #{{true}}]}} []
  (s/valid? ::document-changes {input}))
"#
        ));
    }
    for (index, input) in [
        "nil",
        "false",
        r#""create""#,
        "creation",
        "{:entry creation}",
        "[nil]",
        "[::s/invalid]",
        "[[:create-file creation]]",
    ]
    .into_iter()
    .enumerate()
    {
        body.push_str(&format!(
            r#"
(defn invalid-{index} {{:t/sig '[:-> #{{::s/invalid}}]}} []
  (s/conform ::document-changes {input}))
(defn invalid-{index}? {{:t/sig '[:-> #{{false}}]}} []
  (s/valid? ::document-changes {input}))
"#
        ));
    }
    let report = check(&body);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn ordered_conformance_and_validation_stop_on_the_first_invalid_or_throwing_entry() {
    let mut body = String::new();
    for (label, form) in [
        ("vector", "[%a %b]"),
        ("list", "(list %a %b)"),
        ("seq", "(seq [%a %b])"),
        ("entry", "(clojure.lang.MapEntry/create %a %b)"),
    ] {
        let invalid_first = form.replace("%a", "nil").replace("%b", "overflow");
        let throws_first = form.replace("%a", "overflow").replace("%b", "nil");
        for (op, suffix, invalid) in [
            ("s/conform", "conform", "::s/invalid"),
            ("s/valid?", "valid", "false"),
        ] {
            body.push_str(&format!(
                r#"
(defn {label}-{suffix}-invalid {{:t/sig '[:-> #{{{invalid}}}]}} []
  (try ({op} ::document-changes {invalid_first})
       (catch ArithmeticException _ :unexpected-overflow)))
(defn {label}-{suffix}-overflow {{:t/sig '[:-> #{{:overflow}}]}} []
  (try ({op} ::document-changes {throws_first})
       (catch ArithmeticException _ :overflow)))
"#
            ));
        }
    }
    let report = check(&body);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn optional_workspace_field_is_projected_and_rebuilds_the_outer_entry() {
    let body = r#"
(defn absent {:t/sig '[:-> {}]} [] (s/conform ::workspace-edit {}))
(defn projected {:t/sig '[:-> #{"file:///new"}]} []
  (:uri (first (:document-changes
                (s/conform ::workspace-edit {:document-changes [creation rename]})))))
(defn empty-projection {:t/sig '[:-> (s/tuple (s/coll-of map? :kind seq?) nil?)]} []
  (let [result (:document-changes (s/conform ::workspace-edit {:document-changes []}))]
    [result (seq result)]))
(defn nested-invalid {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::workspace-edit {:document-changes [nil]}))
(defn nested-overflow {:t/sig '[:-> #{:overflow}]} []
  (try (s/conform ::workspace-edit {:document-changes [overflow]})
       (catch ArithmeticException _ :overflow)))
(defn retained-entry {:t/sig '[:-> {}]} []
  (key (s/conform ::workspaces (clojure.lang.MapEntry/create {} {}))))
(defn rebuilt-entry {:t/sig '[:-> (s/tuple {} (s/coll-of map? :kind seq?) nil?)]} []
  (let [raw (clojure.lang.MapEntry/create {} {:document-changes []})
        result (s/conform ::workspaces raw)
        projected (:document-changes (first (rest result)))]
    [(first result) projected (seq projected)]))
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{body}
(defn must-rebuild {{:t/sig '[:-> any?]}} []
  (key (s/conform ::workspaces
                  (clojure.lang.MapEntry/create {{}} {{:document-changes []}}))))
"#
    ));
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert!(finding.message.contains("key"), "{finding:#?}");
}

#[test]
fn other_projection_bodies_and_indirect_sets_keep_explicit_gaps() {
    let body = r#"
(defn select-operation {:t/sig '[any? :-> any?]} [input] input)
(s/def ::wrong-element
  (s/and (s/coll-of ::document-changes-entry) (s/conformer #(map first %))))
(s/def ::wrong-arity
  (s/and (s/coll-of ::document-changes-entry) (s/conformer #(map second % []))))
(s/def ::dynamic
  (s/and (s/coll-of ::document-changes-entry) (s/conformer #(map select-operation %))))
(defn wrong-element {:t/sig '[:-> any?]} [] (s/conform ::wrong-element [creation]))
(defn wrong-arity {:t/sig '[:-> any?]} [] (s/conform ::wrong-arity [creation]))
(defn dynamic {:t/sig '[:-> any?]} [] (s/conform ::dynamic [creation]))
(defn unordered {:t/sig '[:-> any?]} []
  (let [changes #{creation rename}]
    (s/conform ::document-changes changes)))
"#;
    let source = format!("{}{PROJECTION}{INPUTS}{body}", specs());
    let report = Session::default().check("document_changes.clj", &source);
    assert!(!report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .all(|finding| finding.code == "PNX9001"),
        "{report:#?}"
    );
    for key in [
        "::wrong-element",
        "::wrong-arity",
        "::dynamic",
        "::document-changes",
    ] {
        let call_site = source.rfind(key).unwrap();
        assert!(
            report
                .diagnostics
                .iter()
                .any(|finding| finding.span.start.offset == call_site),
            "{key}: {report:#?}"
        );
    }
}

#[test]
fn a_local_map_definition_does_not_replace_the_qualified_core_projection() {
    let projection = PROJECTION.replacen(
        "(s/def ::document-changes",
        "(defn map {:t/sig '[any? any? :-> any?]} [_ items] items)\n(s/def ::document-changes",
        1,
    );
    let source = format!(
        r#"{}
{projection}
{INPUTS}
(s/def ::core-projection
  (s/and (s/coll-of ::document-changes-entry)
         (s/conformer #(clojure.core/map second %))))
(defn local-map {{:t/sig '[:-> any?]}} []
  (s/conform ::document-changes [creation]))
(defn core-map {{:t/sig '[:-> #{{"file:///new"}}]}} []
  (:uri (first (s/conform ::core-projection [creation]))))
"#,
        specs()
    );
    let report = Session::default().check("document_changes.clj", &source);
    assert!(!report.complete, "{report:#?}");
    assert!(report.diagnostics.iter().all(|item| item.code == "PNX9001"));
    assert!(report.diagnostics.iter().any(|item| {
        item.message.contains("supported conversion")
            && &source[item.span.start.offset..item.span.end.offset] == "::document-changes"
    }));
    assert!(report.diagnostics.iter().all(|item| {
        &source[item.span.start.offset..item.span.end.offset] != "::core-projection"
    }));
}

#[test]
fn source_free_workspace_signature_takes_unprojected_edit_collections() {
    let provider = format!(
        r#"{}
{PROJECTION}
(defn accept {{:t/sig '[::workspace-edit :-> keyword?]}} [_] :accepted)
"#,
        specs()
    );
    let exported = Session::default().export_externs(
        &[Source {
            path: "provider.clj",
            source: &provider,
        }],
        &[],
    );
    let edn = exported
        .edn
        .unwrap_or_else(|| panic!("{:#?}", exported.report));
    let check = |argument: &str| {
        let consumer = format!(
            r#"(ns consumer (:require [demo.document-changes-entry :as wire]))
(defn send {{:t/sig '[:-> keyword?]}} [] (wire/accept {argument}))
"#
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
    for input in [
        "{}",
        "{:document-changes []}",
        r#"{:document-changes [{:kind "create" :uri "file:///new"}
                               {:text-document {:uri "file:///old" :version 1N} :edits []}]}"#,
    ] {
        let report = check(input);
        assert!(report.ok && report.complete, "{input}: {report:#?}");
    }

    for input in [
        "{:document-changes nil}",
        r#"{:document-changes [[:create-file {:kind "create" :uri "file:///new"}]]}"#,
        r#"{:document-changes [{:text-document {:uri "file:///old" :version 2147483648} :edits []}]}"#,
    ] {
        let rejected = check(input);
        let [finding] = rejected.files[0].diagnostics.as_slice() else {
            panic!("{input}: {rejected:#?}");
        };
        assert_eq!(finding.code, "PNX2322", "{input}: {finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().path.as_deref(),
            Some("provider.edn")
        );
    }
}

use parenox_analysis::{Report, Session, Source};

const INPUTS: &str = r#"
(def edit {:new-text "text"
           :range {:start {:line 2N :character 2N}
                   :end {:line 2N :character 2N}}})
(def overflow-edit {:new-text "text"
                    :range {:start {:line 2147483648 :character 0}
                            :end {:line 1 :character 0}}})
(def beyond-long-edit {:new-text "text"
                       :range {:start {:line 9223372036854775808N :character 0}
                               :end {:line 1 :character 0}}})
"#;

fn specs() -> &'static str {
    include_str!("../../../tests/fixtures/spec-workspace-changes.clj")
        .split_once(";; JVM-only")
        .unwrap()
        .0
}

fn check(body: &str) -> Report {
    Session::default().check(
        "workspace_changes.clj",
        &format!("{}{INPUTS}{body}", specs()),
    )
}

#[test]
fn a_closed_string_dictionary_keeps_its_keys_and_normalizes_each_edit_value() {
    let report = check(
        r#"
(defn empty-map {:t/sig '[:-> {}]} [] (s/conform ::changes {}))
(defn empty-valid {:t/sig '[:-> #{true}]} [] (s/valid? ::changes {}))
(defn canonical
  {:t/sig '[:-> (s/tuple (s/and int? #{2}) #{"text"} (s/coll-of map? :kind vector?))]} []
  (let [result (s/conform ::changes {"file:///old" [edit] "file:///other" []})
        start (get-in result ["file:///old" 0 :range :start])]
    [(:line start) (get-in result ["file:///old" 0 :new-text]) (get result "file:///other")]))
(defn string-key {:t/sig '[:-> #{"file:///old"}]} []
  (key (first (s/conform ::changes {"file:///old" [edit]}))))
(defn valid {:t/sig '[:-> #{true}]} []
  (s/valid? ::changes {"file:///old" [edit] "file:///other" []}))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn a_wrong_key_stops_before_its_value_and_nonmaps_fail() {
    let mut body = r#"
(defn wrong-key {:t/sig '[:-> #{::s/invalid}]} []
  (try (s/conform ::changes {:bad-key [overflow-edit]})
       (catch ArithmeticException _ :unexpected)))
(defn wrong-key-valid {:t/sig '[:-> #{false}]} []
  (try (s/valid? ::changes {:bad-key [overflow-edit]})
       (catch ArithmeticException _ :unexpected)))
(defn arithmetic {:t/sig '[:-> #{:arithmetic}]} []
  (try (s/conform ::changes {"file:///old" [overflow-edit]})
       (catch ArithmeticException _ :arithmetic)))
(defn illegal-argument {:t/sig '[:-> #{:argument}]} []
  (try (s/valid? ::changes {"file:///old" [beyond-long-edit]})
       (catch IllegalArgumentException _ :argument)))
"#
    .to_owned();
    for (index, input) in [
        "nil",
        "false",
        r#""file:///old""#,
        r#"[["file:///old" [edit]]]"#,
        r#"(clojure.lang.MapEntry/create "file:///old" [edit])"#,
        r#"{"file:///old" :not-edits}"#,
        r#"{"file:///old" [{:new-text "missing range"}]}"#,
    ]
    .into_iter()
    .enumerate()
    {
        body.push_str(&format!(
            r#"
(defn invalid-{index} {{:t/sig '[:-> #{{::s/invalid}}]}} []
  (s/conform ::changes {input}))
(defn invalid-{index}? {{:t/sig '[:-> #{{false}}]}} []
  (s/valid? ::changes {input}))
"#
        ));
    }
    let report = check(&body);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn across_entries_both_rejection_and_overflow_are_possible() {
    let mixed = r#"{:bad-key [overflow-edit] "file:///old" [overflow-edit]}"#;
    let body = format!(
        r#"
(defn conform-order {{:t/sig '[:-> #{{::s/invalid :overflow}}]}} []
  (try (s/conform ::changes {mixed})
       (catch ArithmeticException _ :overflow)))
(defn valid-order {{:t/sig '[:-> #{{false :overflow}}]}} []
  (try (s/valid? ::changes {mixed})
       (catch ArithmeticException _ :overflow)))
"#
    );
    let report = check(&body);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{body}
(defn never-throws {{:t/sig '[:-> #{{::s/invalid}}]}} []
  (try (s/conform ::changes {mixed})
       (catch ArithmeticException _ :overflow)))
(defn always-throws {{:t/sig '[:-> #{{:overflow}}]}} []
  (try (s/valid? ::changes {mixed})
       (catch ArithmeticException _ :overflow)))
"#
    ));
    let [invalid_only, throws_only] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    for (finding, name) in [
        (invalid_only, "never-throws"),
        (throws_only, "always-throws"),
    ] {
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().name,
            format!("demo.workspace-changes/{name}")
        );
    }
}

#[test]
fn a_map_entry_is_preserved_only_when_both_input_maps_are_unchanged() {
    let body = r#"
(s/def ::change-maps (s/coll-of ::changes))
(defn unchanged-map {:t/sig '[:-> (s/and int? #{2})]} []
  (let [canonical (s/conform ::changes {"file:///old" [edit]})
        entry (clojure.lang.MapEntry/create canonical {})]
    (get-in (key (s/conform ::change-maps entry)) ["file:///old" 0 :range :start :line])))
(defn retained-empty {:t/sig '[:-> {}]} []
  (val (s/conform ::change-maps (clojure.lang.MapEntry/create {} {}))))
(defn normalized {:t/sig '[:-> (s/and int? #{2})]} []
  (let [entry (clojure.lang.MapEntry/create {"file:///old" [edit]} {})
        rebuilt (s/conform ::change-maps entry)]
    (get-in (first rebuilt) ["file:///old" 0 :range :start :line])))
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{body}
(defn rebuilt {{:t/sig '[:-> any?]}} []
  (key (s/conform ::change-maps
                  (clojure.lang.MapEntry/create {{"file:///old" [edit]}} {{}}))))
"#
    ));
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert!(finding.message.contains("key"), "{finding:#?}");
}

#[test]
fn a_workspace_edit_conforms_both_optional_fields_when_present() {
    let specs = include_str!("../../../tests/fixtures/spec-document-changes.clj")
        .split_once(";; JVM-only")
        .unwrap()
        .0
        .replacen(
            "(s/def :create-file/options",
            "(s/def ::overwrite any?)\n(s/def ::ignore-if-exists any?)\n(s/def :create-file/options",
            1,
        );
    let body = r#"
(defn combined {:t/sig '[:-> (s/tuple (s/and int? #{2}) #{"file:///new"})]} []
  (let [result (s/conform ::workspace-edit
                         {:changes {"file:///old" [edit]}
                          :document-changes [{:kind "create" :uri "file:///new"}]})]
    [(get-in result [:changes "file:///old" 0 :range :start :line])
     (:uri (first (:document-changes result)))]))
(defn dictionary-only {:t/sig '[:-> #{"text"}]} []
  (get-in (s/conform ::workspace-edit {:changes {"file:///old" [edit]}})
          [:changes "file:///old" 0 :new-text]))
(defn bad-dictionary {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::workspace-edit {:changes {:bad-key [overflow-edit]} :document-changes []}))
"#;
    let report =
        Session::default().check("document_changes.clj", &format!("{specs}{INPUTS}{body}"));
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn conformance_of_an_open_or_broad_dictionary_keeps_an_explicit_gap() {
    let body = r#"
(defn open-map {:t/sig '[map? :-> any?]} [value] (s/conform ::changes value))
(defn known-dictionary-domain {:t/sig '[::changes :-> any?]} [value]
  (s/conform ::changes value))
"#;
    let source = format!("{}{INPUTS}{body}", specs());
    let report = Session::default().check("workspace_changes.clj", &source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    for finding in &report.diagnostics {
        assert_eq!(finding.code, "PNX9001", "{finding:#?}");
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            "::changes"
        );
    }
}

#[test]
fn a_dictionary_contract_exports_and_imports_without_provider_source() {
    let provider = format!(
        r#"{}
(defn accept {{:t/sig '[::changes :-> keyword?]}} [_] :accepted)
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
            r#"(ns consumer (:require [demo.workspace-changes :as wire]))
{INPUTS}
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
    for input in ["{}", r#"{"file:///old" [edit]}"#] {
        let accepted = check(input);
        assert!(accepted.ok && accepted.complete, "{input}: {accepted:#?}");
    }
    for input in [
        "{:bad-key [edit]}",
        r#"{"file:///old" [overflow-edit]}"#,
        r#"{"file:///old" [{:new-text "missing range"}]}"#,
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

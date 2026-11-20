use parenox_analysis::{Report, Session, Source};

const OPTIONS: &str = r#"
(s/def ::overwrite any?)
(s/def ::ignore-if-exists any?)
"#;

const INPUTS: &str = r#"
(def create-input {:kind "create" :uri "file:///new" :options {:overwrite false}})
(def rename-input {:kind "rename" :old-uri "file:///old" :new-uri "file:///new"})
(def text-input
  {:text-document {:uri "file:///old" :version 1N}
   :edits [{:new-text "x" :range {:start {:line 2N :character 0}
                                :end {:line 2N :character 1}}}]})
"#;

fn specs() -> &'static str {
    include_str!("../../../tests/fixtures/spec-document-changes-entry.clj")
        .split_once(";; JVM-only")
        .unwrap()
        .0
}

fn check(body: &str) -> Report {
    Session::default().check(
        "document_changes_entry.clj",
        &format!("{}{OPTIONS}{INPUTS}{body}", specs()),
    )
}

#[test]
fn raw_or_returns_the_matching_tag_and_conformed_value_as_a_map_entry() {
    let mut body = r#"
(s/def ::marker-tag (s/or ::s/invalid ::create-file))
(defn marker-tag {:t/sig '[:-> (s/tuple #{::s/invalid} #{"file:///new"})]} []
  (let [entry (s/conform ::marker-tag create-input)]
    [(key entry) (:uri (val entry))]))
(defn marker-valid {:t/sig '[:-> #{true}]} [] (s/valid? ::marker-tag create-input))
(defn create {:t/sig '[:-> (s/tuple #{:create-file} #{"file:///new"} #{false})]} []
  (let [entry (s/conform ::document-changes-entry create-input)]
    [(key entry) (:uri (val entry)) (:overwrite (:options (val entry)))]))
(defn rename {:t/sig '[:-> (s/tuple #{:rename-file} #{"file:///old"} #{"file:///new"})]} []
  (let [entry (s/conform ::document-changes-entry rename-input)]
    [(key entry) (:old-uri (val entry)) (:new-uri (val entry))]))
(defn text-edit {:t/sig '[:-> (s/tuple #{:text-document-edit} int? int? #{"x"})]} []
  (let [entry (s/conform ::document-changes-entry text-input)
        edit (first (:edits (val entry)))]
    [(key entry) (:version (:text-document (val entry)))
     (:line (:start (:range edit))) (:new-text edit)]))
(defn valid-create {:t/sig '[:-> #{true}]} [] (s/valid? ::document-changes-entry create-input))
(defn valid-rename {:t/sig '[:-> #{true}]} [] (s/valid? ::document-changes-entry rename-input))
(defn valid-text-edit {:t/sig '[:-> #{true}]} [] (s/valid? ::document-changes-entry text-input))
"#
    .to_owned();
    for (index, input) in [
        "nil",
        "false",
        "::s/invalid",
        "[]",
        "[:create-file create-input]",
        "{}",
        r#"{:kind "rename" :uri "file:///new"}"#,
        r#"{:text-document {:uri "file:///old" :version nil} :edits []}"#,
    ]
    .into_iter()
    .enumerate()
    {
        body.push_str(&format!(
            r#"
(defn invalid-{index} {{:t/sig '[:-> #{{::s/invalid}}]}} []
  (s/conform ::document-changes-entry {input}))
(defn invalid-{index}? {{:t/sig '[:-> #{{false}}]}} []
  (s/valid? ::document-changes-entry {input}))
"#
        ));
    }
    let report = check(&body);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn first_success_prevents_later_overflow_and_an_earlier_overflow_escapes() {
    let report = check(
        r#"
(def overlap {:kind "create" :uri "file:///new"
              :text-document {:uri "file:///old" :version 2147483648}
              :edits []})
(def falling-through {:kind "other" :uri "file:///new"
                      :text-document {:uri "file:///old" :version 2147483648}
                      :edits []})
(defn create-first {:t/sig '[:-> #{:create-file}]} []
  (try (first (s/conform ::document-changes-entry overlap))
       (catch ArithmeticException _ :unexpected-overflow)))
(defn create-valid {:t/sig '[:-> #{true}]} []
  (try (s/valid? ::document-changes-entry overlap)
       (catch ArithmeticException _ :unexpected-overflow)))
(defn text-first {:t/sig '[:-> #{:overflow}]} []
  (try (s/conform ::text-document-first overlap)
       (catch ArithmeticException _ :overflow)))
(defn fallback-overflow {:t/sig '[:-> #{:overflow}]} []
  (try (s/conform ::document-changes-entry falling-through)
       (catch ArithmeticException _ :overflow)))
(defn validation-overflow {:t/sig '[:-> #{:overflow}]} []
  (try (s/valid? ::document-changes-entry falling-through)
       (catch ArithmeticException _ :overflow)))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn wrapping_a_map_entry_in_each_branch_rebuilds_the_outer_collection() {
    let body = r#"
(s/def ::wrapper (s/keys :req-un [::document-changes-entry]))
(s/def ::wrapped (s/coll-of ::wrapper))
(defn nested-entries
  {:t/sig '[:-> (s/tuple #{:create-file} #{"file:///new"} #{:rename-file} #{"file:///old"})]} []
  (let [left {:document-changes-entry create-input}
        right {:document-changes-entry rename-input}
        result (s/conform ::wrapped (clojure.lang.MapEntry/create left right))
        created (:document-changes-entry (first result))
        renamed (:document-changes-entry (first (rest result)))]
    [(key created) (:uri (val created)) (key renamed) (:old-uri (val renamed))]))
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{body}
(defn rebuilt {{:t/sig '[:-> any?]}} []
  (let [left {{:document-changes-entry create-input}}
        right {{:document-changes-entry rename-input}}]
    (key (s/conform ::wrapped (clojure.lang.MapEntry/create left right)))))
"#
    ));
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert!(finding.message.contains("key"), "{finding:#?}");
}

#[test]
fn later_branch_registration_must_precede_the_first_possible_resolution() {
    let declarations = r#"
(s/def ::candidate (s/or :create ::create-file :future ::late-map))
"#;
    let early_resolution = r#"
(s/def ::probe
  (s/with-gen string?
    (try (s/conform ::candidate create-input)
         (catch Exception _ (fn [] nil)))))
"#;
    let late_branch = "(s/def ::late-map (s/keys :req-un [::version]))";
    let caller = r#"
(defn later {:t/sig '[:-> #{:create}]} []
  (key (s/conform ::candidate create-input)))
"#;
    let complete = check(&format!("{declarations}{late_branch}{caller}"));
    assert!(complete.ok && complete.complete, "{complete:#?}");

    let source = format!(
        "{}{OPTIONS}{INPUTS}{declarations}{early_resolution}{late_branch}{caller}",
        specs()
    );
    let blocked = Session::default().check("document_changes_entry.clj", &source);
    let call_site = source.rfind("::candidate").unwrap();
    assert!(!blocked.complete, "{blocked:#?}");
    assert!(
        blocked.diagnostics.iter().any(|finding| {
            finding.code == "PNX9001"
                && finding.message.contains("supported conversion")
                && finding.span.start.offset == call_site
        }),
        "{blocked:#?}"
    );
}

#[test]
fn source_free_contract_retains_the_input_maps_and_excludes_tagged_outputs() {
    let provider = format!(
        r#"{}
{OPTIONS}
(defn accept {{:t/sig '[::document-changes-entry :-> keyword?]}} [_] :accepted)
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
        r#"{:kind "create" :uri "file:///new"}"#,
        r#"{:kind "rename" :old-uri "file:///old" :new-uri "file:///new"}"#,
        r#"{:text-document {:uri "file:///old" :version 1N} :edits []}"#,
    ] {
        let report = check(input);
        assert!(report.ok && report.complete, "{input}: {report:#?}");
    }

    for input in [
        r#"[:create-file {:kind "create" :uri "file:///new"}]"#,
        r#"{:kind "rename" :uri "file:///new"}"#,
        r#"{:text-document {:uri "file:///old" :version 2147483648} :edits []}"#,
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

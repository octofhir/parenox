use parenox::{DocumentHover, Report, Session, Source};

const MODEL: &str = include_str!("../examples/map_boundary/src/map_boundary/model.clj");
const RESPONSE: &str = include_str!("../examples/map_boundary/src/map_boundary/response.clj");
const HTTP: &str = include_str!("../examples/map_boundary/src/map_boundary/http.clj");
const EXTERNS: Source = Source {
    path: "externs.edn",
    source: include_str!("../examples/map_boundary/externs.edn"),
};
const MEDLEY: Source = Source {
    path: "medley.edn",
    source: include_str!("fixtures/medley-context/externs.edn"),
};

fn sources<'a>(response: &'a str, http: &'a str) -> [Source<'a>; 3] {
    [
        Source {
            path: "model.clj",
            source: MODEL,
        },
        Source {
            path: "response.clj",
            source: response,
        },
        Source {
            path: "http.clj",
            source: http,
        },
    ]
}

fn check(extra: &str) -> Report {
    Session::default().check("examples.clj", &format!("{RESPONSE}\n{extra}"))
}

#[test]
fn nested_callback_context_tracks_contract_edits_and_eager_map_fields() {
    let consumer = Source {
        path: "consumer.clj",
        source: r#"(ns nested.consumer (:require [nested.options :as o]))
            (defn use-options {:t/sig '[:-> string?]} []
              (o/render {:stage {:run (fn [label] (do (inc label) "ok")) :guard "ready"}}
                        (o/input)))"#,
    };
    let provider = Source {
        path: "options.clj",
        source: r#"(ns nested.options (:require [clojure.spec.alpha :as s]))
            (s/def ::input string?)
            (defn render
              {:t/sig '[{:stage {:run (s/fspec :args (s/cat :value ::input) :ret string?)
                                 :guard string?}} ::input :-> string?]}
              [options value] ((get-in options [:stage :run]) value))
            (defn input {:t/sig '[:-> ::input]} [] "text")"#,
    };
    let mut session = Session::default();
    let original = session.check_project(&[consumer, provider], None);
    let [finding] = original.files[0].diagnostics.as_slice() else {
        panic!("{original:#?}");
    };
    assert!(original.complete, "{original:#?}");
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("String"));
    assert_eq!(
        &consumer.source[finding.span.start.offset..finding.span.end.offset],
        "label"
    );

    let source = provider
        .source
        .replace("::input string?", "::input number?")
        .replace("\"text\"", "7");
    let edited = Source {
        source: &source,
        ..provider
    };
    let warm = session.check_project(&[consumer, edited], None);
    let cold = Session::default().check_project(&[consumer, edited], None);
    assert!(
        warm.ok && warm.complete && cold.ok && cold.complete,
        "{warm:#?}\n{cold:#?}"
    );
    let reverted = session.check_project(&[consumer, provider], None);
    assert_eq!(
        serde_json::to_value(&reverted.files[0].diagnostics).unwrap(),
        serde_json::to_value(&original.files[0].diagnostics).unwrap()
    );

    let eager = consumer.source.replace("\"ready\"", "(subs nil 0)");
    let report = session.check_project(
        &[
            Source {
                source: &eager,
                ..consumer
            },
            provider,
        ],
        None,
    );
    let [finding] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        &eager[finding.span.start.offset..finding.span.end.offset],
        "nil"
    );
}

#[test]
fn a_nested_fixed_callback_has_a_distinct_path_from_a_generic_result_callback() {
    let source = r#"(ns demo.callback-paths (:require [clojure.spec.alpha :as s]))
        (defn run
          {:t/sig '(t/all [a b]
            [{:run (s/fspec :args (s/cat :value a) :ret b)
              :stage {:run (s/fspec :args (s/cat :value number?) :ret string?)}}
             a :-> b])}
          [options value]
          (do ((get-in options [:stage :run]) 1) ((:run options) value)))
        (defn render {:t/sig '[:-> number?]} []
          (run {:run #(count %) :stage {:run (fn [n] (str (inc n)))}} "text"))"#;
    let report = Session::default().check("callback-paths.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn map_callbacks_use_generic_input_types_from_other_arguments() {
    let source = r#"(ns demo.generic-options (:require [clojure.spec.alpha :as s]))
        (defn render
          {:t/sig '(t/all [a] [{:run (s/fspec :args (s/cat :value a) :ret string?)} a :-> string?])}
          [options value] ((:run options) value))
        (defn length-text {:t/sig '[string? :-> string?]} [text]
          (render {:run #(str (count %))} text))
        (defn invalid {:t/sig '[string? :-> string?]} [text]
          (let [key :run]
            (render {key (fn [label] (do (inc label) "ok"))} text)))"#;
    let mut session = Session::default();
    let report = session.check("options.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("String"));
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "label"
    );
    let Some(DocumentHover::Local(hover)) =
        session.document_hover_at("options.clj", finding.span.start.offset)
    else {
        panic!("Expected the callback parameter type at the operand");
    };
    assert_eq!(hover.inferred_type, "String");
}

#[test]
fn an_eager_field_can_supply_the_type_for_a_callback_in_the_same_map() {
    let source = r#"(ns demo.same-options (:require [clojure.spec.alpha :as s]))
        (defn render
          {:t/sig '(t/all [a] [{:run (s/fspec :args (s/cat :value a) :ret string?) :value a} :-> string?])}
          [options] ((:run options) (:value options)))
        (defn length-text {:t/sig '[string? :-> string?]} [text]
          (render {:run #(str (count %)) :value text}))
        (defn invalid {:t/sig '[string? :-> string?]} [text]
          (render {:run (fn [label] (do (inc label) "ok")) :value text}))"#;
    let report = Session::default().check("same-options.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("String"));
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "label"
    );
}

#[test]
fn a_required_map_callback_can_supply_the_calls_result_type() {
    let source = r#"(ns demo.result-options (:require [clojure.spec.alpha :as s]))
        (defn render
          {:t/sig '(t/all [a b] [{:run (s/fspec :args (s/cat :value a) :ret b) :value a} :-> b])}
          [options] ((:run options) (:value options)))
        (defn count-text {:t/sig '[string? :-> number?]} [text] (count text))
        (defn via-inline {:t/sig '[string? :-> number?]} [text]
          (render {:run #(count %) :value text}))
        (defn via-named {:t/sig '[string? :-> number?]} [text]
          (render {:run count-text :value text}))
        (defn use-renderer
          {:t/sig '[(s/fspec :args (s/cat :options {:run (s/fspec :args (s/cat :x string?) :ret number?)
                                                 :value string?})
                             :ret number?)
                    string? :-> number?]}
          [renderer text]
          (renderer {:run count-text :value text}))
        (defn via-specialization {:t/sig '[string? :-> number?]} [text]
          (use-renderer render text))
        (defn wrong-result {:t/sig '[string? :-> string?]} [text]
          (render {:run #(count %) :value text}))"#;
    let report = Session::default().check("result-options.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert!(
        finding
            .message
            .contains("Return value of demo.result-options/wrong-result")
    );
    assert_eq!(
        finding.actual.as_deref(),
        Some("JVM fixed-precision integer in the JVM int range")
    );
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "(render {:run #(count %) :value text})"
    );
}

#[test]
fn exported_nested_callback_uses_a_sibling_input_and_its_own_result() {
    let provider = Source {
        path: "catalog.clj",
        source: r#"(ns catalog (:require [clojure.spec.alpha :as s]))
            (defn render
              {:t/sig '(t/all [a b]
                [{:stage {:run (s/fspec :args (s/cat :value a) :ret b)}
                  :value a} :-> b])}
              [options] ((get-in options [:stage :run]) (:value options)))"#,
    };
    let export = Session::default().export_externs(&[provider], &[]);
    let edn = export.edn.unwrap_or_else(|| panic!("{:?}", export.report));
    let good = r#"(ns app (:require [catalog :as c]))
        (defn label-count {:t/sig '[string? :-> number?]} [label] (count label))
        (defn via-inline {:t/sig '[:-> number?]} []
          (c/render {:stage {:run (fn [label] (count label))} :value "text"}))
        (defn via-named {:t/sig '[:-> number?]} []
          (c/render {:stage {:run label-count} :value "text"}))"#;
    let check = |source| {
        Session::default().check_project(
            &[Source {
                path: "app.clj",
                source,
            }],
            Some(Source {
                path: "catalog.edn",
                source: &edn,
            }),
        )
    };
    let report = check(good);
    assert!(report.ok && report.complete, "{report:#?}");

    let bad = good.replace("(fn [label] (count label))", "(fn [label] (inc label))");
    let report = check(&bad);
    let [finding] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("String"));
    assert_eq!(
        &bad[finding.span.start.offset..finding.span.end.offset],
        "label"
    );
    assert_eq!(
        finding
            .callback_contract
            .as_ref()
            .and_then(|c| c.path.as_deref()),
        Some("catalog.edn")
    );
}

#[test]
fn two_nested_callbacks_cannot_supply_the_same_result_parameter() {
    let report = check(
        "(defn ambiguous
          {:t/sig '(t/all [a b]
            [{:stage {:first (s/fspec :args (s/cat :x a) :ret b)
                      :second (s/fspec :args (s/cat :x a) :ret b)}} a :-> b])}
          [options value] ((get-in options [:stage :first]) value))",
    );
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
}

#[test]
fn map_callback_context_refreshes_after_an_input_contract_edit() {
    let consumer = Source {
        path: "consumer.clj",
        source: r#"(ns demo.consumer (:require [demo.options :as o]))
            (defn render {:t/sig '[:-> string?]} []
              (o/render {:run (fn [label] (do (inc label) "ok"))} (o/input)))"#,
    };
    let provider = Source {
        path: "options.clj",
        source: r#"(ns demo.options (:require [clojure.spec.alpha :as s]))
            (defn render
              {:t/sig '(t/all [a] [{:run (s/fspec :args (s/cat :value a) :ret string?)} a :-> string?])}
              [options value] ((:run options) value))
            (defn input {:t/sig '[:-> string?]} [] "text")"#,
    };
    let changed = provider
        .source
        .replace(r#"[:-> string?]} [] "text""#, "[:-> number?]} [] 7");
    let mut session = Session::default();
    let original = [consumer, provider];
    let warm = session.check_project(&original, None);
    let [finding] = warm.files[0].diagnostics.as_slice() else {
        panic!("{warm:#?}");
    };
    assert!(warm.complete, "{warm:#?}");
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("String"));

    let edited = [
        consumer,
        Source {
            source: &changed,
            ..provider
        },
    ];
    let accepted = session.check_project(&edited, None);
    let cold = Session::default().check_project(&edited, None);
    assert!(accepted.ok && accepted.complete, "{accepted:#?}");
    assert!(cold.ok && cold.complete, "{cold:#?}");
    let restored = session.check_project(&original, None);
    assert_eq!(
        serde_json::to_value(&restored.files[0].diagnostics).unwrap(),
        serde_json::to_value(&warm.files[0].diagnostics).unwrap(),
    );

    let exported = Session::default().export_externs(&[provider], &[]);
    let snapshot = exported
        .edn
        .unwrap_or_else(|| panic!("{:#?}", exported.report));
    let imported = Session::default().check_project(
        &[consumer],
        Some(Source {
            path: "options.edn",
            source: &snapshot,
        }),
    );
    let [imported_finding] = imported.files[0].diagnostics.as_slice() else {
        panic!("{imported:#?}");
    };
    assert!(imported.complete, "{imported:#?}");
    assert_eq!(imported_finding.code, finding.code);
    assert_eq!(imported_finding.span, finding.span);
    assert_eq!(imported_finding.actual, finding.actual);
}

#[test]
fn checks_a_storage_extern_through_to_the_response_contract() {
    let report = Session::default().check_project(&sources(RESPONSE, HTTP), Some(EXTERNS));
    assert!(report.ok && report.complete, "{report:#?}");

    let wrong_body = HTTP.replace("response/ok render-name", "response/ok (fn [_] {:body 42})");
    let report = Session::default().check_project(&sources(RESPONSE, &wrong_body), Some(EXTERNS));
    let [diag] = report.files[2].diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diag.code, "PNX2322");
    assert!(diag.actual.as_deref().unwrap().contains(":body: Number"));
}

#[test]
fn an_extern_that_loses_the_required_row_does_not_satisfy_the_response_builder() {
    let mut session = Session::default();
    let files = sources(RESPONSE, HTTP);
    assert!(session.check_project(&files, Some(EXTERNS)).ok);
    let renamed_field = EXTERNS.source.replace("{:row ", "{:rows ");
    let externs = Source {
        source: &renamed_field,
        ..EXTERNS
    };
    let report = session.check_project(&files, Some(externs));
    assert_eq!(report.files[2].stats.checked_functions, 1);
    assert_eq!(report.files[2].stats.reused_functions, 1);
    assert_eq!(report.files[1].stats.checked_functions, 0);
    let diagnostics = &report.files[2].diagnostics;
    assert!(
        diagnostics.iter().any(|d| d.code == "PNX2322"),
        "{report:#?}"
    );
    let missing = diagnostics
        .iter()
        .find(|d| d.code == "PNX9010")
        .expect("missing row evidence");
    assert!(missing.message.contains("required field :row"));
}

#[test]
fn renaming_generic_fields_contract_parameters_keeps_passing_callers_cached() {
    let mut session = Session::default();
    assert!(
        session
            .check_project(&sources(RESPONSE, HTTP), Some(EXTERNS))
            .ok
    );
    let renamed = RESPONSE
        .replace("[a b]", "[input output]")
        .replace(":row a", ":row input")
        .replace(":name a", ":name input")
        .replace("keyword? a", "keyword? input")
        .replace(":ret b", ":ret output")
        .replace(":-> b", ":-> output")
        .replace(":body b", ":body output");
    let report = session.check_project(&sources(&renamed, HTTP), Some(EXTERNS));
    assert!(report.ok && report.complete, "{report:#?}");
    assert_eq!(report.files[1].stats.checked_functions, 2);
    assert_eq!(report.files[2].stats.checked_functions, 0);
    assert_eq!(report.files[2].stats.reused_functions, 2);
}

#[test]
fn required_field_evidence_preserves_nil_but_does_not_invent_an_absent_field() {
    let passing = check(
        "(defn explicit-nil {:t/sig '[:-> {:status number? :body nil?}]} []
           (ok (fn [x] {:body x}) {:row nil}))
         (defn string-key {:t/sig '[{\"row\" string?} :-> string?]} [entry] (get entry \"row\"))",
    );
    assert!(passing.ok && passing.complete, "{passing:#?}");

    let missing = check(
        "(defn absent {:t/sig '[:-> {:status number? :body nil?}]} []
           (ok (fn [x] {:body x}) {}))",
    );
    assert!(!missing.complete, "{missing:#?}");
    assert!(
        missing
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX9010" && d.message.contains("required field :row"))
    );
}

#[test]
fn dictionary_evidence_includes_literal_fields_and_typed_additional_values() {
    let report = check(
        "(defn mixed {:t/sig '[(s/map-of keyword? string?) :-> number?]} [labels]
           (select-name #(inc %) (assoc labels :display-name 7)))",
    );
    assert!(
        report.diagnostics.iter().any(|d| {
            d.code == "PNX2322"
                && d.message.contains("inc")
                && d.actual.as_deref() == Some("Number | String")
        }),
        "{report:#?}"
    );
}

#[test]
fn return_types_can_contain_a_generic_dictionary_value() {
    let report = check(
        "(defn singleton
           {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :x a) :ret b)
                                 (s/map-of keyword? a) :-> (s/map-of #{:label} b)])}
           [f row] {:label (f (req! row :input))})
         (defn render {:t/sig '[:-> (s/map-of keyword? string?)]} []
           (singleton #(str (inc %)) {:input 7}))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn an_empty_dictionary_does_not_supply_a_value_type() {
    let report =
        check("(defn missing {:t/sig '[:-> string?]} [] (select-name (fn [x] (str x)) {}))");
    assert!(!report.complete);
    let [diag] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diag.code, "PNX9010");
    assert!(diag.message.contains("this map's values"));
}

#[test]
fn a_named_builder_specializes_from_structured_callback_inputs_and_outputs() {
    let report = check(
        "(defn invoke-builder
           {:t/sig '[(s/fspec :args (s/cat :render (s/fspec :args (s/cat :x number?) :ret {:body string?})
                                           :result {:row number?})
                              :ret {:status number? :body string?})
                     :-> {:status number? :body string?}]}
           [builder] (builder (fn [n] {:body (str (inc n))}) {:row 5}))
         (defn built {:t/sig '[:-> {:status number? :body string?}]} [] (invoke-builder ok))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn structured_callbacks_can_pass_a_map_field_and_return_a_vector_element() {
    let report = check(
        "(defn convert-row
           {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :item {:value a}) :ret (s/coll-of b :kind vector?))
                                 a :-> (s/coll-of b :kind vector?)])}
           [f x] (f {:value x}))
         (defn render {:t/sig '[:-> (s/coll-of string? :kind vector?)]} []
           (convert-row (fn [row] [(str (inc (:value row)))]) 7))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn a_callback_returning_the_wrong_shape_cannot_supply_a_record_field() {
    let report = check(
        "(defn missing-body {:t/sig '[:-> {:status number? :body string?}]} []
           (ok (fn [_] {}) {:row \"Ada\"}))",
    );
    assert!(!report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX9010" && d.message.contains("parameter b"))
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX2322" && d.message.contains("Argument 1"))
    );
}

#[test]
fn nested_parameters_and_extra_record_fields_are_rejected() {
    for signature in [
        "(t/all [a] [a :-> (s/map-of (s/coll-of a :kind vector?) string?)])",
        "(t/all [a] [a :-> (s/map-of a (s/coll-of a :kind vector?))])",
        "(t/all [a] [a :-> {:nested {:row a}}])",
        "(t/all [a b] [{:first (s/fspec :args (s/cat :x a) :ret b)
                        :second (s/fspec :args (s/cat :x a) :ret b) :value a} :-> b])",
    ] {
        let report = check(&format!("(defn unsupported {{:t/sig '{signature}}} [x] x)"));
        let [diag] = report.diagnostics.as_slice() else {
            panic!("{report:#?}")
        };
        assert_eq!(diag.code, "PNX9001");
    }
    let extra = check(
        "(defn extra {:t/sig '[:-> {:status number? :body string?}]} [] (ok (fn [x] {:body x}) {:row \"x\" :extra 1}))",
    );
    let [diag] = extra.diagnostics.as_slice() else {
        panic!("{extra:#?}")
    };
    assert_eq!(diag.code, "PNX2322");
}

#[test]
fn dictionary_key_evidence_uses_possible_entries_and_ignores_absence_masks() {
    let source = r#"(ns app (:require [clojure.spec.alpha :as s] [medley.core :as m]))
(defn forward
  {:t/sig '(t/all [k v] [(s/map-of k v) :-> (s/map-of k v)])}
  [entries]
  (m/map-kv-vals (fn [_ value] value) (dissoc entries :omitted)))
(defn choose
  {:t/sig '[boolean? :-> (s/map-of #{:required :optional} string?)]}
  [flag]
  (m/map-kv-vals (fn [_ value] value)
    (if flag {:required "a" :optional "b"} {:required "a"})))
"#;
    let report = Session::default().check_project(
        &[Source {
            path: "app.clj",
            source,
        }],
        Some(MEDLEY),
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn empty_and_open_dictionaries_do_not_establish_callback_types() {
    let source = r#"(ns app (:require [medley.core :as m]))
(defn empty-input {:t/sig '[:-> map?]} []
  (m/map-kv-vals (fn [key value] value) {}))
(defn open-input {:t/sig '[map? :-> map?]} [entries]
  (m/map-kv-vals (fn [key value] value) entries))
"#;
    let report = Session::default().check_project(
        &[Source {
            path: "app.clj",
            source,
        }],
        Some(MEDLEY),
    );
    let diagnostics = &report.files[0].diagnostics;
    assert_eq!(diagnostics.len(), 4, "{report:#?}");
    for (input, findings) in ["{}", "entries"]
        .into_iter()
        .zip(diagnostics.as_chunks::<2>().0)
    {
        for (part, diagnostic) in ["keys", "values"].into_iter().zip(findings) {
            assert_eq!(diagnostic.code, "PNX9010");
            assert!(diagnostic.message.ends_with(&format!("this map's {part}")));
            assert_eq!(
                &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
                input
            );
        }
    }
}

#[test]
fn exported_dictionary_contracts_specialize_without_the_provider() {
    let provider = Source {
        path: "catalog.clj",
        source: r#"(ns catalog (:require [clojure.spec.alpha :as s]))
(defn label-keys
  {:t/sig '(t/all [(k :< string?) v] [(s/map-of k v) :-> (s/map-of k v)])}
  [entries] entries)
(defn same-type
  {:t/sig '(t/all [t] [(s/map-of t t) :-> (s/map-of t t)])}
  [entries] entries)
"#,
    };
    let mut session = Session::default();
    let export = session.export_externs(&[provider], &[]);
    let edn = export.edn.unwrap_or_else(|| panic!("{:?}", export.report));
    let externs = Source {
        path: "catalog.edn",
        source: &edn,
    };
    let good = r#"(ns app (:require [catalog :as c] [clojure.spec.alpha :as s]))
(defn retain {:t/sig '[:-> (s/map-of #{"row"} nil?)]} [] (c/label-keys {"row" nil}))
(defn same {:t/sig '[:-> (s/map-of string? string?)]} [] (c/same-type {"row" "label"}))
"#;
    let check = |source| {
        Session::default().check_project(
            &[Source {
                path: "app.clj",
                source,
            }],
            Some(externs),
        )
    };
    let report = check(good);
    assert!(report.ok && report.complete, "{report:#?}");
    let bad = good
        .replace("#{\"row\"}", "keyword?")
        .replace("{\"row\" nil}", "{:row nil}")
        .replace("(s/map-of string? string?)", "(s/map-of number? number?)")
        .replace("{\"row\" \"label\"}", "{\"row\" 1}");
    let report = check(&bad);
    let diagnostics = &report.files[0].diagnostics;
    assert_eq!(diagnostics.len(), 2, "{report:#?}");
    assert!(diagnostics.iter().all(|d| d.code == "PNX2322"
        && d.contract.as_ref().and_then(|c| c.path.as_deref()) == Some("catalog.edn")));
    assert!(
        diagnostics[0]
            .message
            .contains("Bound for type parameter k")
    );
    assert!(diagnostics[1].message.contains("Argument 1"));
}

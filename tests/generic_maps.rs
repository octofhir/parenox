use parenox::{Report, Session, Source};

const MODEL: &str = include_str!("../examples/map_boundary/src/map_boundary/model.clj");
const RESPONSE: &str = include_str!("../examples/map_boundary/src/map_boundary/response.clj");
const HTTP: &str = include_str!("../examples/map_boundary/src/map_boundary/http.clj");
const EXTERNS: Source = Source {
    path: "externs.edn",
    source: include_str!("../examples/map_boundary/externs.edn"),
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
fn generic_map_keys_nested_parameters_and_extra_record_fields_are_rejected() {
    for signature in [
        "(t/all [a] [a :-> (s/map-of a string?)])",
        "(t/all [a] [a :-> {:nested {:row a}}])",
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

use parenox::Session;

const PRELUDE: &str = "(ns demo (:require [clojure.spec.alpha :as s] [clojure.string :as str]))\n\
(s/def :patient/id string?)\n\
(s/def :patient/name string?)\n\
(s/def ::patient (s/keys :req-un [:patient/id] :opt-un [:patient/name]))\n";

fn source(body: &str) -> String {
    format!("{PRELUDE}{body}")
}

fn assert_ok(body: &str) {
    let report = Session::default().check("test.clj", &source(body));
    assert!(report.ok && report.complete, "{:#?}", report.diagnostics);
}

#[test]
fn deeply_nested_conditionals_and_bindings_fit_on_a_worker_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            for (open, leaf, close) in [
                ("(if true ", "\"text\"", " nil)"),
                ("(let [text \"text\"] ", "text", ")"),
            ] {
                let body = format!("{}{leaf}{}", open.repeat(125), close.repeat(125));
                assert_ok(&format!("(defn deep {{:t/sig '[:-> string?]}} [] {body})"));
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn optional_field_requires_nil_handling() {
    let report = Session::default().check(
        "test.clj",
        &source("(defn label {:t/sig '[::patient :-> string?]} [p] (str/upper-case (:name p)))"),
    );
    assert!(!report.ok);
    assert!(report.complete);
    assert_eq!(report.diagnostics.len(), 1);
    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.expected.as_deref(), Some("Non-nil value"));
    assert!(diagnostic.actual.as_deref().unwrap().contains("Nil"));
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|n| n.contains(":name is optional"))
    );
}

#[test]
fn or_default_removes_nil() {
    assert_ok(
        "(defn label {:t/sig '[::patient :-> string?]} [p] (str/upper-case (or (:name p) \"Unknown\")))",
    );
}

#[test]
fn required_field_is_not_nilable() {
    assert_ok("(defn id {:t/sig '[::patient :-> string?]} [p] (str/upper-case (:id p)))");
}

#[test]
fn predicate_narrows_destructured_local() {
    assert_ok(
        "(defn label {:t/sig '[::patient :-> string?]} [{:keys [name]}] (if (some? name) (str/upper-case name) \"Unknown\"))",
    );
}

#[test]
fn nil_predicate_narrows_else_branch() {
    assert_ok(
        "(defn label {:t/sig '[(s/nilable string?) :-> string?]} [x] (if (nil? x) \"Unknown\" (str/upper-case x)))",
    );
}

#[test]
fn truthiness_narrows_locals() {
    assert_ok(
        "(defn label {:t/sig '[(s/nilable string?) :-> string?]} [x] (if x (str/upper-case x) \"Unknown\"))",
    );
}

#[test]
fn zero_is_truthy_in_clojure() {
    assert_ok("(defn f {:t/sig '[:-> number?]} [] (if 0 1 \"unreachable\"))");
}

#[test]
fn boolean_uses_jvm_cast_results_for_literals_contracts_and_apply() {
    assert_ok(
        "(defn f {:t/sig '[:-> (s/tuple #{false} #{false} #{true} #{true} #{true})]} []
           [(boolean nil) (apply boolean [false]) (boolean 0) (boolean []) (boolean {})])
         (defn from-nullable-false {:t/sig '[(s/nilable #{false}) :-> #{false}]} [v] (boolean v))
         (defn from-any {:t/sig '[any? :-> boolean?]} [v] (boolean v))
         (defn shadow {:t/sig '[:-> #{true}]} []
           (let [boolean (fn [_] true)] (boolean false)))",
    );

    let wrong = source("(defn f {:t/sig '[:-> #{true}]} [] (boolean false))");
    let report = Session::default().check("test.clj", &wrong);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("false"));
    assert_eq!(
        &wrong[finding.span.start.offset..finding.span.end.offset],
        "(boolean false)"
    );
}

#[test]
fn or_preserves_falsey_nil_and_false_semantics() {
    assert_ok("(defn f {:t/sig '[:-> string?]} [] (or false nil \"value\"))");
}

#[test]
fn and_does_not_discard_false_from_the_result() {
    let report = Session::default().check(
        "test.clj",
        &source("(defn f {:t/sig '[boolean? :-> string?]} [x] (and x \"value\"))"),
    );
    assert!(report.diagnostics.iter().any(|d| d.code == "PNX2322"));
}

#[test]
fn return_type_and_call_arguments_are_checked_independently() {
    let report = Session::default().check(
        "test.clj",
        &source(
            "(defn f {:t/sig '[number? :-> number?]} [x] \"wrong\")\n\
         (defn g {:t/sig '[:-> number?]} [] (f \"wrong\"))",
        ),
    );
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|d| d.code == "PNX2322")
            .count(),
        2
    );
}

#[test]
fn forward_calls_and_qualified_local_calls_work() {
    assert_ok(
        "(defn g {:t/sig '[number? :-> number?]} [x] (demo/f x))\n\
               (defn f {:t/sig '[number? :-> number?]} [x] (+ x 1))",
    );
}

#[test]
fn lexical_bindings_shadow_project_functions() {
    assert_ok(
        "(defn f {:t/sig '[number? :-> number?]} [x] x)\n\
               (defn g {:t/sig '[string? :-> string?]} [f] f)",
    );
}

#[test]
fn local_predicate_name_is_not_used_for_narrowing() {
    let report = Session::default().check("test.clj", &source(
        "(defn f {:t/sig '[(s/nilable string?) boolean? :-> string?]} [x some?] (if (some? x) (str/upper-case x) \"fallback\"))",
    ));
    assert!(!report.ok);
    assert!(!report.complete);
}

#[test]
fn let_bindings_are_sequential_and_scoped() {
    assert_ok(
        "(defn f {:t/sig '[::patient :-> string?]} [p] (let [n (:name p) label (or n \"Unknown\")] (str/upper-case label)))",
    );
}

#[test]
fn missing_required_map_field_is_an_error() {
    let report = Session::default().check(
        "test.clj",
        &source(
            "(defn id {:t/sig '[::patient :-> string?]} [p] (:id p))\n\
         (defn f {:t/sig '[:-> string?]} [] (id {}))",
        ),
    );
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].code, "PNX2322");
}

#[test]
fn optional_map_fields_still_have_value_types() {
    let report = Session::default().check(
        "test.clj",
        &source(
            "(defn id {:t/sig '[::patient :-> string?]} [p] (:id p))\n\
         (defn f {:t/sig '[:-> string?]} [] (id {:id \"1\" :name 42}))",
        ),
    );
    assert!(report.diagnostics.iter().any(|d| d.code == "PNX2322"));
}

#[test]
fn absent_optional_field_on_closed_literal_is_allowed() {
    assert_ok(
        "(defn id {:t/sig '[::patient :-> string?]} [p] (:id p))\n\
               (defn f {:t/sig '[:-> string?]} [] (id {:id \"1\"}))",
    );
}

#[test]
fn get_default_does_not_replace_an_explicit_nil() {
    let report = Session::default().check(
        "test.clj",
        &source("(defn f {:t/sig '[:-> string?]} [] (get {:name nil} :name \"fallback\"))"),
    );
    assert_eq!(report.diagnostics[0].actual.as_deref(), Some("Nil"));
}

#[test]
fn get_default_replaces_absence() {
    assert_ok("(defn f {:t/sig '[:-> string?]} [] (get {} :name \"fallback\"))");
}

#[test]
fn assoc_makes_optional_field_required() {
    assert_ok(
        "(defn f {:t/sig '[::patient :-> string?]} [p] (str/upper-case (:name (assoc p :name \"Alice\"))))",
    );
}

#[test]
fn aliases_auto_keywords_and_qualified_keys_are_resolved() {
    assert_ok(
        "(s/def ::id string?)\n\
               (s/def ::entity (s/keys :req [::id]))\n\
               (defn f {:t/sig '[::entity :-> string?]} [e] (::id e))",
    );
}

#[test]
fn alias_only_spec_and_test_macros_do_not_create_checked_declarations() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as-alias s] [clojure.test :as-alias t]))
(s/def ::message string?)
(defn echo {:t/sig '[:-> ::message]} [] 42)
(t/deftest checks (t/is (= \"\" (subs false 0))))
"#;
    let mut session = Session::default();
    let report = session.check("demo.clj", source);
    let gaps: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.code == "PNX9003")
        .map(|finding| &source[finding.span.start.offset..finding.span.end.offset])
        .collect();
    assert_eq!(gaps, ["s/def", "t/deftest"], "{report:#?}");
    assert!(!report.diagnostics.iter().any(|item| item.code == "PNX2322"));

    let loaded = source.replace(":as-alias", ":as");
    let report = session.check("demo.clj", &loaded);
    let mismatches: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.code == "PNX2322")
        .map(|finding| &loaded[finding.span.start.offset..finding.span.end.offset])
        .collect();
    assert_eq!(mismatches, ["42", "false"], "{report:#?}");
    assert!(!report.diagnostics.iter().any(|item| item.code == "PNX9003"));
}

#[test]
fn vectors_are_checked_structurally() {
    assert_ok("(defn f {:t/sig '[:-> (s/coll-of string? :kind vector?)]} [] [\"a\" \"b\"])");
    let report = Session::default().check(
        "test.clj",
        &source("(defn f {:t/sig '[:-> (s/coll-of string? :kind vector?)]} [] [\"a\" 1])"),
    );
    assert!(!report.ok);
}

#[test]
fn unsupported_code_is_not_silently_accepted() {
    let report = Session::default().check(
        "test.clj",
        &source("(defn f {:t/sig '[:-> string?]} [] (str '(#demo/opaque {:value 1})))"),
    );
    assert!(!report.ok);
    assert!(!report.complete);
}

#[test]
fn open_map_undeclared_key_is_unknown_not_proven_absent() {
    let report = Session::default().check(
        "test.clj",
        &source("(defn f {:t/sig '[::patient :-> string?]} [p] (:nmae p))"),
    );
    assert!(!report.complete);
    assert_eq!(report.diagnostics[0].code, "PNX9003");
}

#[test]
fn recursive_specs_report_a_gap_without_recursing_forever() {
    let report =
        Session::default().check("test.clj", &source("(s/def ::node (s/keys :opt [::node]))"));
    assert!(!report.complete);
}

#[test]
fn deleting_spec_does_not_leave_a_runtime_registry_entry() {
    let mut session = Session::default();
    assert!(
        session
            .check(
                "test.clj",
                &source("(defn f {:t/sig '[::patient :-> string?]} [p] (:id p))",)
            )
            .ok
    );
    let modified = source("(defn f {:t/sig '[::patient :-> string?]} [p] (:id p))")
        .replace("(s/def :patient/id string?)", "");
    assert!(!session.check("test.clj", &modified).complete);
}

#[test]
fn unchanged_functions_are_reused_but_edited_bodies_are_checked() {
    let mut session = Session::default();
    let text =
        source("(defn f {:t/sig '[:-> number?]} [] 1)\n(defn g {:t/sig '[:-> number?]} [] 2)");
    assert_eq!(session.check("test.clj", &text).stats.checked_functions, 2);
    assert_eq!(session.check("test.clj", &text).stats.reused_functions, 2);
    let edited = text.replace("[] 2)", "[] 3)");
    let report = session.check("test.clj", &edited);
    assert!(report.ok);
    assert_eq!(report.stats.reused_functions, 1);
    assert_eq!(report.stats.checked_functions, 1);
}

#[test]
fn interface_changes_invalidate_callers() {
    let mut session = Session::default();
    let text = source(
        "(defn f {:t/sig '[number? :-> number?]} [x] x)\n\
                       (defn g {:t/sig '[:-> number?]} [] (f 1))",
    );
    assert!(session.check("test.clj", &text).ok);
    let edited = text.replace("'[number? :-> number?]", "'[string? :-> string?]");
    let report = session.check("test.clj", &edited);
    assert!(!report.ok);
    assert_eq!(report.stats.checked_functions, 2);
    assert_eq!(report.stats.reused_functions, 0);
}

#[test]
fn moving_a_function_updates_cached_diagnostic_ranges() {
    let mut session = Session::default();
    let text = source("(defn f {:t/sig '[:-> number?]} [] \"wrong\")");
    let before = session.check("test.clj", &text);
    let after = session.check("test.clj", &format!("\n{text}"));
    assert_eq!(after.stats.reused_functions, 0);
    assert_eq!(
        after.diagnostics[0].span.start.line,
        before.diagnostics[0].span.start.line + 1
    );
    assert_ne!(before.source_hash, after.source_hash);
}

#[test]
fn unicode_positions_are_scalar_columns_and_byte_offsets() {
    let report = Session::default().check(
        "test.clj",
        &source("(defn имя {:t/sig '[:-> number?]} [] \"ошибка\")"),
    );
    let span = report.diagnostics[0].span;
    assert!(span.start.offset > span.start.column);
    let text = source("(defn имя {:t/sig '[:-> number?]} [] \"ошибка\")");
    assert_eq!(&text[span.start.offset..span.end.offset], "\"ошибка\"");
}

#[test]
fn malformed_input_produces_a_structured_error() {
    let report = Session::default().check("test.clj", "(defn broken [x]");
    assert!(!report.ok && !report.complete);
    assert_eq!(report.diagnostics[0].code, "PNX9000");
}

#[test]
fn nested_reader_discards_do_not_execute_or_check_discarded_code() {
    assert_ok(
        "#_ #_ (unsupported) (also-unsupported)\n\
               (defn f {:t/sig '[:-> number?]} [] 1)",
    );
}

#[test]
fn nan_symbol_is_not_mistaken_for_a_numeric_literal() {
    let report = Session::default().check(
        "test.clj",
        &source("(defn f {:t/sig '[:-> number?]} [] NaN)"),
    );
    assert!(report.diagnostics.iter().any(|d| d.code == "PNX2304"));
}

#[test]
fn unreachable_forms_still_need_symbol_resolution() {
    for body in [
        "(if true 1 missing)",
        "(or 1 missing)",
        "(and false missing)",
    ] {
        let report = Session::default().check(
            "test.clj",
            &source(&format!("(defn f {{:t/sig '[:-> number?]}} [] {body})",)),
        );
        assert!(
            report.diagnostics.iter().any(|d| d.code == "PNX2304"),
            "{body}"
        );
    }
}

#[test]
fn deeply_nested_input_is_rejected_without_overflowing_the_stack() {
    let text = format!("{}nil{}", "(".repeat(300), ")".repeat(300));
    let report = Session::default().check("test.clj", &text);
    assert!(!report.complete);
    assert_eq!(report.diagnostics[0].code, "PNX9001");
    assert!(report.diagnostics[0].message.contains("nesting limit"));
}

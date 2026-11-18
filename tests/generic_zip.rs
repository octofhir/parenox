use parenox::{Report, Session, Source};

const EXAMPLE: &str = include_str!("../examples/generic_zip.clj");

fn check(extra: &str) -> Report {
    Session::default().check("zip.clj", &format!("{EXAMPLE}\n{extra}"))
}

#[test]
fn takes_two_vector_element_types_and_specializes_inline_and_named_callbacks() {
    let report = check(
        "(defn labels {:t/sig '[:-> (s/coll-of string? :kind vector?)]} []
           (zip-with (fn [x n] (str (str/trim x) (inc n))) [\"label\"] [42]))
         (defn via-apply {:t/sig '[:-> (s/coll-of {:label string? :value boolean?} :kind vector?)]} []
           (apply zip-with [reading [\"online\"] [false]]))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn takes_independent_parameters_from_the_fields_of_one_record() {
    let report = check(
        "(defn rearrange
           {:t/sig '(t/all [a b] [{:left a :right b} :-> {:left b :right a}])}
           [row] {:left (:right row) :right (:left row)})
         (defn reordered {:t/sig '[:-> {:left string? :right nil?}]} []
           (rearrange {:left nil :right \"known\"}))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn callback_result_and_input_roles_follow_use_not_binder_order() {
    let report = check(
        "(defn forward
           {:t/sig '(t/all [result a b]
                      [(s/fspec :args (s/cat :x a :y b) :ret result)
                       (s/coll-of a :kind vector?) (s/coll-of b :kind vector?)
                       :-> (s/coll-of result :kind vector?)])}
           [f xs ys] (zip-with f xs ys))
         (defn measured {:t/sig '[:-> (s/coll-of number? :kind vector?)]} []
           (forward (fn [text n] (+ (count text) n)) [\"x\"] [7]))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn an_ordinary_argument_takes_precedence_over_a_callback_result_for_the_same_parameter() {
    let report = check(
        "(defn convert
           {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :x a) :ret b) a b :-> b])}
           [f x _] (f x))
         (defn bad {:t/sig '[:-> string?]} []
           (convert (fn [n] (inc n)) 7 \"string example\"))",
    );
    let [diag] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diag.code, "PNX2322");
    assert!(diag.message.contains("Return value of callback"));
    assert_eq!(diag.expected.as_deref(), Some("String"));
}

#[test]
fn missing_first_input_does_not_hide_a_definite_error_in_the_second_callback_argument() {
    let report = check(
        "(defn bad {:t/sig '[:-> (s/coll-of number? :kind vector?)]} []
           (zip-with (fn [_ text] (inc text)) [] [\"wrong\"]))",
    );
    assert!(!report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX9010" && d.message.contains("parameter a"))
    );
    assert!(
        report.diagnostics.iter().any(|d| d.code == "PNX2322"
            && d.message.contains("inc")
            && d.actual.as_deref() == Some("String")),
        "{report:#?}"
    );
}

#[test]
fn each_parameter_has_an_independent_bound_and_evidence_site() {
    let report = check(
        "(defn limited
           {:t/sig '(t/all [(a :< number?) (b :< string?)]
                      [a (s/coll-of b :kind vector?) :-> a])}
           [x _] x)
         (defn bad {:t/sig '[:-> number?]} [] (limited \"not a number\" []))",
    );
    let [first, second] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(first.code, "PNX2322");
    assert!(first.message.contains("Bound for type parameter a"));
    assert_eq!(second.code, "PNX9010");
    assert!(second.message.contains("parameter b"));
}

#[test]
fn a_call_with_swapped_inputs_does_not_equate_the_rigid_parameters() {
    let report = check(
        "(defn wrong {:t/sig '(t/all [a b] [a b :-> a])} [left right]
           (wrong right left))",
    );
    let [diag] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diag.code, "PNX2322");
    assert_eq!(diag.expected.as_deref(), Some("a"));
    assert_eq!(diag.actual.as_deref(), Some("b"));
}

#[test]
fn nullable_results_do_not_lose_the_element_type_or_hide_the_need_for_a_guard() {
    let report = check(
        "(defn wrong {:t/sig '[:-> string?]} []
           (str/trim (find-value {:weekly \"later\"} :daily)))",
    );
    let [diag] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert!(report.complete);
    assert_eq!(diag.code, "PNX2322");
    assert_eq!(diag.actual.as_deref(), Some("Nil | String"));
}

#[test]
fn a_callback_can_supply_a_concrete_type_for_a_nullable_return() {
    let report = check(
        "(defn convert-if
           {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :x a) :ret b) a boolean? :-> (s/nilable b)])}
           [f x keep] (if keep (f x) nil))
         (defn run {:t/sig '[boolean? :-> (s/nilable string?)]} [keep]
           (convert-if #(str (inc %)) 5 keep))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn a_nullable_callback_result_alone_is_not_used_to_guess_a_parameter() {
    let report = check(
        "(defn ambiguous
           {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :x a) :ret (s/nilable b)) a :-> (s/nilable b)])}
           [f x] (f x))",
    );
    let [diag] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diag.code, "PNX9001");
}

#[test]
fn externs_with_three_parameters_reuse_callers_after_renaming_but_not_a_bound_edit() {
    let source = Source {
        path: "app.clj",
        source: "(ns app (:require [vendor :as v]))
          (defn score {:t/sig '[:-> number?]} [] (v/combine (fn [text n] (+ (count text) n)) \"abc\" 1))",
    };
    let externs = Source {
        path: "externs.edn",
        source: "{vendor/combine
          (t/all [a (b :< number?) c]
            [(clojure.spec.alpha/fspec :args (clojure.spec.alpha/cat :x a :y b) :ret c) a b :-> c])}",
    };
    let mut session = Session::default();
    assert!(session.check_project(&[source], Some(externs)).ok);
    let renamed = externs
        .source
        .replace("[a (b :< number?) c]", "[x (y :< number?) z]")
        .replace(
            ":x a :y b) :ret c) a b :-> c]",
            ":x x :y y) :ret z) x y :-> z]",
        );
    let warmed = session.check_project(
        &[source],
        Some(Source {
            source: &renamed,
            ..externs
        }),
    );
    assert!(warmed.ok && warmed.complete, "{warmed:#?}");
    assert_eq!(warmed.stats.reused_functions, 1);
    let changed = renamed.replace("y :< number?", "y :< string?");
    let report = session.check_project(
        &[source],
        Some(Source {
            source: &changed,
            ..externs
        }),
    );
    assert_eq!(report.stats.checked_functions, 1);
    assert!(
        report.files[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX2322" && d.message.contains("parameter y"))
    );
}

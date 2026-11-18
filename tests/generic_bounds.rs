use parenox::{Report, Session, Source};

fn check(extra: &str) -> Report {
    let example = include_str!("../examples/generic_callbacks.clj");
    Session::default().check("bounds.clj", &format!("{example}\n{extra}"))
}

#[test]
fn a_bound_can_be_used_but_does_not_erase_the_rigid_type() {
    let report = check(
        "(defn checked {:t/sig '(t/all [(a :< number?)] [a :-> a])} [value]
           (if (nil? value) nil (if (number? value) value nil)))
         (defn next-value {:t/sig '(t/all [(a :< number?)] [a :-> number?])} [value]
           (if value (inc value) nil))
         (defn not-the-same {:t/sig '(t/all [(a :< number?)] [a :-> a])} [value]
           (inc value))",
    );
    let [diag] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diag.code, "PNX2322");
    assert!(diag.message.contains("not-the-same"));
    assert_eq!(diag.expected.as_deref(), Some("a"));
    assert_eq!(diag.actual.as_deref(), Some("Number"));
}

#[test]
fn checks_both_bounds_at_the_evidence_sites() {
    let report = check(
        "(defn bad-input {:t/sig '[:-> ::names]} [] (numeric-labels #(str %) [1 \"x\"]))
         (defn bad-output {:t/sig '[:-> ::names]} [] (numeric-labels #(inc %) [1]))",
    );
    let [input, output] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    for (diag, param, expected, actual) in [
        (input, "a", "Number", "Number | String"),
        (output, "b", "String", "Number"),
    ] {
        assert_eq!(diag.code, "PNX2322");
        assert!(
            diag.message
                .contains(&format!("Bound for type parameter {param}"))
        );
        assert_eq!(diag.expected.as_deref(), Some(expected));
        assert_eq!(diag.actual.as_deref(), Some(actual));
        assert_eq!(
            diag.contract.as_ref().unwrap().name,
            "demo.generic-callbacks/numeric-labels"
        );
    }
}

#[test]
fn a_bound_does_not_fill_in_missing_evidence() {
    let report =
        check("(defn empty-labels {:t/sig '[:-> ::names]} [] (numeric-labels number-label []))");
    assert!(!report.complete);
    let [diag] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diag.code, "PNX9010");
    assert!(diag.message.contains("parameter a"));
}

#[test]
fn string_bounds_support_reading_without_losing_the_original_type() {
    let report = check(
        "(defn string-size {:t/sig '(t/all [(s :< string?)] [s :-> number?])} [text]
           (count text))
         (defn string-first {:t/sig '(t/all [(s :< string?)] [s :-> (s/nilable char?)])} [text]
           (first text))
         (defn string-index {:t/sig '(t/all [(s :< string?)] [s :-> (s/nilable char?)])} [text]
           (get text 0))
         (defn characters {:t/sig '(t/all [(s :< string?)] [s :-> (s/coll-of char? :kind vector?)])} [text]
           (mapv identity text))
         (defn original {:t/sig '(t/all [(s :< string?)] [s :-> s])} [text]
           (or (not-empty text) text))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn named_generic_callbacks_specialize_independently_in_direct_and_apply_calls() {
    let report = check(
        "(defn call-string {:t/sig '[(s/fspec :args (s/cat :x string?) :ret string?) :-> string?]}
           [f] (f \"text\"))
         (defn via-concrete {:t/sig '[:-> string?]} [] (call-string forward))
         (defn via-inference {:t/sig '[:-> number?]} [] (convert forward 9))
         (defn via-apply {:t/sig '[:-> ::names]} [] (apply map-values [forward [\"x\"]]))
         (defn forwarded-bound
           {:t/sig '(t/all [(a :< number?)] [a :-> string?])} [x]
           (convert number-label x))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn named_callback_neither_ignores_its_bound_nor_infers_from_the_expected_result() {
    let report = check(
        "(defn bad-bound {:t/sig '[:-> ::names]} [] (map-values number-label [\"text\"]))
         (defn bad-result {:t/sig '[:-> ::names]} [] (map-values forward [7]))",
    );
    let [input, output] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(input.code, "PNX2322");
    assert!(input.message.contains("Argument 1"));
    assert_eq!(output.code, "PNX2322");
    assert_eq!(output.actual.as_deref(), Some("Vector<Number>"));
}

#[test]
fn a_callback_argument_can_supply_the_second_parameter_of_a_named_generic_callback() {
    let report = check(
        "(defn call-converter
           {:t/sig '[(s/fspec :args (s/cat :f (s/fspec :args (s/cat :x number?) :ret string?)
                                           :value number?) :ret string?) :-> string?]}
           [convert-it] (convert-it (fn [x] (str x)) 1))
         (defn converted {:t/sig '[:-> string?]} [] (call-converter convert))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn a_concrete_callback_return_does_not_override_the_nested_evidence() {
    let report = check(
        "(defn call-converter
           {:t/sig '[(s/fspec :args (s/cat :f (s/fspec :args (s/cat :x number?) :ret string?)
                                           :value number?) :ret number?) :-> number?]}
           [convert-it] (convert-it (fn [x] (str x)) 1))
         (defn wrong-converter {:t/sig '[:-> number?]} [] (call-converter convert))",
    );
    let [diag] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert!(report.complete);
    assert_eq!(diag.code, "PNX2322");
    assert!(diag.message.contains("Argument 1"));
}

#[test]
fn nested_return_evidence_does_not_bypass_callback_input_checks() {
    let report = check(
        "(defn call-converter
           {:t/sig '[(s/fspec :args (s/cat :f (s/fspec :args (s/cat :x string?) :ret string?)
                                           :value number?) :ret string?) :-> string?]}
           [convert-it] (convert-it (fn [x] x) 1))
         (defn wrong-callback {:t/sig '[:-> string?]} [] (call-converter convert))",
    );
    let [diag] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert!(report.complete);
    assert_eq!(diag.code, "PNX2322");
    assert!(diag.message.contains("Argument 1"));
}

#[test]
fn changing_only_an_extern_bound_rechecks_its_callers() {
    let app = Source {
        path: "app.clj",
        source: "(ns app (:require [vendor :as v]))
          (defn display {:t/sig '[:-> number?]} [] (v/same 12))",
    };
    let before = Source {
        path: "externs.edn",
        source: "{vendor/same (t/all [(a :< number?)] [a :-> a])}",
    };
    let mut session = Session::default();
    assert!(session.check_project(&[app], Some(before)).ok);
    let after = Source {
        source: "{vendor/same (t/all [(x :< string?)] [x :-> x])}",
        ..before
    };
    let report = session.check_project(&[app], Some(after));
    assert_eq!(report.stats.checked_functions, 1);
    assert_eq!(
        report.files[0].diagnostics[0].expected.as_deref(),
        Some("String")
    );
}

#[test]
fn malformed_structural_and_dependent_bounds_are_rejected() {
    for binder in [
        "(a :< map?)",
        "(a :< a)",
        "(a :< (s/nilable number?))",
        "(a :> number?)",
    ] {
        let report = check(&format!(
            "(defn bad {{:t/sig '(t/all [{binder}] [a :-> a])}} [x] x)"
        ));
        let [diag] = report.diagnostics.as_slice() else {
            panic!("{report:#?}")
        };
        assert_eq!(diag.code, "PNX9001");
    }
}

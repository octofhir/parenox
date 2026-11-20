use parenox::{Report, Session};

const NS: &str =
    "(ns demo.string-case (:require [clojure.spec.alpha :as s] [clojure.string :as str]))\n";
const OPERATIONS: [&str; 3] = ["upper-case", "lower-case", "capitalize"];

fn check(body: &str) -> Report {
    Session::default().check("string_case.clj", &format!("{NS}{body}"))
}

#[test]
fn case_functions_accept_non_nil_values_in_direct_and_apply_calls() {
    let mut body = String::new();
    for operation in OPERATIONS {
        for (index, input) in [
            "false",
            "true",
            "42",
            "1N",
            "3/2",
            "1.25",
            r"\q",
            ":demo/word",
            "::s/invalid",
            "[1 false]",
            "{:word 1}",
            r#""straße""#,
        ]
        .into_iter()
        .enumerate()
        {
            body.push_str(&format!(
                r#"
(defn {operation}-{index} {{:t/sig '[:-> (s/tuple string? string?)]}} []
  [(str/{operation} {input}) (apply str/{operation} [{input}])])
"#
            ));
        }
    }
    let report = check(&body);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn modeled_callbacks_use_the_same_case_input_contract() {
    for operation in OPERATIONS {
        let body = format!(
            r#"
(defn transform {{:t/sig '[:-> (s/coll-of string? :kind vector?)]}} []
  (mapv str/{operation} [false 42 ::s/invalid ["x"]]))
"#
        );
        let report = check(&body);
        assert!(report.ok && report.complete, "{operation}: {report:#?}");

        let body = body.replace("[false 42 ::s/invalid [\"x\"]]", "[false nil]");
        let report = check(&body);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{operation}: {report:#?}");
        };
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(finding.expected.as_deref(), Some("Non-nil value"));
        assert!(finding.suggestions.is_empty(), "{finding:#?}");
    }
}

#[test]
fn nil_case_inputs_have_an_honest_contract_and_safe_string_fallback() {
    let body = r#"
(defn direct {:t/sig '[:-> string?]} [] (str/upper-case nil))
(defn spread {:t/sig '[:-> string?]} [] (apply str/lower-case [nil]))
(defn false-or-nil {:t/sig '[#{false nil} :-> string?]} [x] (str/capitalize x))
"#;
    let source = format!("{NS}{body}");
    let report = check(body);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 3, "{report:#?}");
    let expected = [
        ("clojure.string/upper-case", "nil"),
        ("clojure.string/lower-case", "nil"),
        ("clojure.string/capitalize", "x"),
    ];
    for (finding, (operation, input)) in report.diagnostics.iter().zip(expected) {
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(finding.expected.as_deref(), Some("Non-nil value"));
        assert!(finding.message.contains(operation), "{finding:#?}");
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            input
        );
    }
    // A false value is already valid. Replacing it with a string would
    // change its result and is not an appropriate Nil default.
    assert!(report.diagnostics[2].suggestions.is_empty(), "{report:#?}");

    let spread = check(
        r#"(defn projected {:t/sig '[(s/tuple (s/nilable string?)) :-> string?]} [args]
             (apply str/upper-case args))"#,
    );
    let [finding] = spread.diagnostics.as_slice() else {
        panic!("{spread:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert_eq!(finding.expected.as_deref(), Some("Non-nil value"));
    assert!(finding.suggestions.is_empty(), "{finding:#?}");

    for operation in OPERATIONS {
        let body = format!(
            r#"
(defn optional {{:t/sig '[(s/nilable string?) :-> string?]}} [input]
  (str/{operation} input))
"#
        );
        let mut source = format!("{NS}{body}");
        let report = check(&body);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{operation}: {report:#?}");
        };
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(finding.expected.as_deref(), Some("Non-nil value"));
        let [suggestion] = finding.suggestions.as_slice() else {
            panic!("{finding:#?}");
        };
        assert_eq!(suggestion.replacement, r#"(or input "")"#);
        let span = suggestion.span;
        assert_eq!(&source[span.start.offset..span.end.offset], "input");
        source.replace_range(span.start.offset..span.end.offset, &suggestion.replacement);
        let repaired = Session::default().check("string_case.clj", &source);
        assert!(repaired.ok && repaired.complete, "{repaired:#?}");
    }
}

#[test]
fn case_results_do_not_assume_a_jvm_locale() {
    let report = check(
        r#"
(defn upper {:t/sig '[:-> #{"I"}]} [] (str/upper-case "i"))
(defn lower {:t/sig '[:-> #{"i"}]} [] (str/lower-case "I"))
(defn cap {:t/sig '[:-> #{"Istanbul"}]} [] (str/capitalize "iSTANBUL"))
"#,
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 3, "{report:#?}");
    for finding in report.diagnostics {
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(finding.actual.as_deref(), Some("String"));
    }
}

#[test]
fn blank_accepts_false_and_nil_while_trim_stays_on_strings() {
    let report = check(
        r#"
(defn blank-false {:t/sig '[:-> #{true}]} [] (str/blank? false))
(defn blank-nil {:t/sig '[:-> #{true}]} [] (apply str/blank? [nil]))
(defn blank-string {:t/sig '[string? :-> boolean?]} [x] (str/blank? x))
(defn blank-falsey {:t/sig '[(s/or :off #{false} :missing nil?) :-> #{true}]} [x]
  (str/blank? x))
(defn trim-nonblank {:t/sig '[(s/or :off #{false} :text string?) :-> string?]} [x]
  (if (str/blank? x) "" (str/trim x)))
(defn trim-string {:t/sig '[string? :-> string?]} [x] (apply str/trim [x]))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");

    let body = r#"
(defn failures {:t/sig '[:-> any?]} []
  (str/blank? true)
  (str/blank? 1)
  (str/trim false)
  (str/trim nil)
  (apply str/trim [42]))
"#;
    let source = format!("{NS}{body}");
    let report = check(body);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 5, "{report:#?}");
    for (index, (finding, input)) in report
        .diagnostics
        .iter()
        .zip(["true", "1", "false", "nil", "42"])
        .enumerate()
    {
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            input
        );
        if index < 2 {
            assert!(
                finding.message.contains("clojure.string/blank?"),
                "{finding:#?}"
            );
        } else {
            assert_eq!(finding.expected.as_deref(), Some("String"));
        }
    }
}

#[test]
fn case_functions_remain_unary_and_unknown_uses_are_incomplete() {
    for operation in OPERATIONS {
        let report = check(&format!(
            r#"
(defn direct {{:t/sig '[:-> string?]}} [] (str/{operation} "a" "b"))
(defn spread {{:t/sig '[:-> string?]}} [] (apply str/{operation} []))
"#
        ));
        assert!(report.complete && !report.ok, "{operation}: {report:#?}");
        assert_eq!(report.diagnostics.len(), 2, "{operation}: {report:#?}");
        assert!(
            report
                .diagnostics
                .iter()
                .all(|finding| finding.code == "PNX2554")
        );
    }
    let report = check(r#"(defn unknown {:t/sig '[any? :-> string?]} [x] (str/capitalize x))"#);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX9004");
    assert!(finding.message.contains("Non-nil value"), "{finding:#?}");

    let report = check(
        r#"(defn guarded {:t/sig '[any? :-> string?]} [x]
             (if (some? x) (str/upper-case x) ""))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(
        r#"(defn through-value {:t/sig '[:-> string?]} []
             (let [f str/upper-case] (f :word)))"#,
    );
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX9003", "{finding:#?}");
}

use parenox::{Session, Source};

const RECORD: &str = "
    (ns records (:require [clojure.spec.alpha :as s]))
    (s/def ::id number?)
    (s/def ::name string?)
    (s/def ::record (s/keys :req-un [::id ::name]))
";

#[test]
fn rejected_arguments_do_not_supply_result_types_to_other_calls() {
    let source = r#"(ns rejected)
(defn encode {:t/sig '[string? :-> string?]} [text] text)
(defn tagged {:t/sig '(t/all [T] [T string? :-> T])} [value tag] value)
(defn direct {:t/sig '[:-> number?]} [] (inc (encode nil)))
(defn spread {:t/sig '[:-> number?]} [] (inc (apply encode [nil])))
(defn generic {:t/sig '[:-> number?]} [] (inc (tagged "text" 0)))
"#;
    let report = Session::default().check("rejected.clj", source);
    assert_eq!(report.diagnostics.len(), 3, "{report:#?}");
    for (diagnostic, owner, label, argument) in [
        (
            &report.diagnostics[0],
            "rejected/encode",
            "Argument 1",
            "nil",
        ),
        (
            &report.diagnostics[1],
            "rejected/encode",
            "Argument 1",
            "nil",
        ),
        (&report.diagnostics[2], "rejected/tagged", "Argument 2", "0"),
    ] {
        assert_eq!(diagnostic.code, "PNX2322");
        assert_eq!(diagnostic.contract.as_ref().unwrap().name, owner);
        assert!(diagnostic.message.starts_with(label));
        assert_eq!(
            &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
            argument
        );
    }
}

#[test]
fn unknown_map_field_does_not_hide_a_wrong_field_at_a_call() {
    let source = format!(
        "{RECORD}
         (defn label {{:t/sig '[::record :-> string?]}} [record] (:name record))
         (defn caller {{:t/sig '[:-> string?]}} []
           (label {{:id (missing) :name 42}}))"
    );
    let report = Session::default().check("records.clj", &source);
    let mismatch = report
        .diagnostics
        .iter()
        .find(|d| d.code == "PNX2322")
        .expect("the known field is incompatible even if id is unknown");
    assert!(!report.ok);
    assert_eq!(
        &source[mismatch.span.start.offset..mismatch.span.end.offset],
        "42"
    );
    assert_eq!(mismatch.expected.as_deref(), Some("String"));
    assert!(mismatch.message.contains("field :name"));
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    assert!(report.diagnostics.iter().any(|d| d.code == "PNX2304"));
}

#[test]
fn unknown_field_in_the_contract_does_not_hide_a_wrong_return_field() {
    let source = format!(
        "{RECORD}
         (defn record {{:t/sig '[:-> ::record]}} [] {{:id 1 :name 42}})"
    )
    .replace("(s/def ::id number?)", "(s/def ::id unsupported?)");
    let report = Session::default().check("records.clj", &source);
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX2322" && d.message.starts_with("Return value"))
    );
}

#[test]
fn incompatible_union_arm_is_not_hidden_by_an_unknown_arm() {
    let report = Session::default().check(
        "records.clj",
        "(defn next-id {:t/sig '[boolean? :-> number?]} [use-source?]
           (inc (if use-source? (missing) \"not an id\")))",
    );
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX2322" && d.actual.as_deref() == Some("Unknown | String"))
    );
}

#[test]
fn incomplete_extern_is_visible_at_a_checked_boundary_and_an_edit_clears_it() {
    let consumer = Source {
        path: "app.clj",
        source: "(ns app (:require [vendor :as v]))
                 (defn must-be-text {:t/sig '[:-> string?]} [] (v/value))
                 (defn shape-only {:t/sig '[:-> map?]} [] {:value (v/value)})
                 (defn unreachable {:t/sig '[:-> string?]} []
                   (if true \"ok\" (clojure.string/upper-case (v/value))))",
    };
    let externs = Source {
        path: "externs.edn",
        source: "{vendor/value [:-> :missing/spec]}",
    };
    let mut session = Session::default();
    let report = session.check_project(&[consumer], Some(externs));
    let file = &report.files[0];
    assert!(!file.ok && !file.complete, "{file:#?}");
    assert_eq!(file.diagnostics.len(), 1, "{file:#?}");
    assert_eq!(file.diagnostics[0].code, "PNX9004");
    assert!(file.diagnostics[0].message.contains("must-be-text"));

    let fixed = session.check_project(
        &[consumer],
        Some(Source {
            source: "{vendor/value [:-> string?]}",
            ..externs
        }),
    );
    assert!(fixed.ok && fixed.complete, "{fixed:#?}");
}

#[test]
fn expected_union_can_have_a_proven_match_or_remain_unproven() {
    let source = "(ns app (:require [clojure.spec.alpha :as s]))
        (s/def ::unsupported unsupported?)
        (defn proven {:t/sig '[:-> (s/or :unknown ::unsupported :text string?)]} [] \"ok\")
        (defn unproven {:t/sig '[:-> (s/or :unknown ::unsupported :text string?)]} [] 42)";
    let report = Session::default().check("app.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    let diag = &report.diagnostics[1];
    assert_eq!(diag.code, "PNX9004");
    assert!(diag.message.contains("Return value of app/unproven"));
}

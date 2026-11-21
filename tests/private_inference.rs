use parenox::Session;

fn check(source: &str) -> parenox::Report {
    Session::default().check("private.clj", source)
}

#[test]
fn helpers_infer_from_typed_calls_and_preserve_collections_and_literal_contracts() {
    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s] [clojure.string :as str]))
(s/def ::status #{"active" "inactive"})
(s/def ::patient (s/keys :opt-un [:patient/name]))
(s/def :patient/name string?)
(defn label {:t/sig '[::patient :-> string?]} [patient] (normalized-name patient))
(defn- normalized-name [{:keys [name]}] (str/upper-case (or name "Unknown")))
(defn pass {:private true} [x] x)
(defn text {:t/sig '[string? :-> string?]} [x] (pass x))
(defn number {:t/sig '[number? :-> number?]} [x] (pass x))
(defn status {:t/sig '[:-> ::status]} [] (pass "active"))
(defn values {:t/sig '[:-> (s/coll-of string? :kind set?)]} [] (pass #{"active"}))
"#;
    let report = check(source);
    assert!(report.ok && report.complete, "{report:#?}");
    let bad = check(&source.replace("(pass \"active\")", "(pass \"deleted\")"));
    assert!(!bad.ok && bad.complete, "{bad:#?}");
    assert_eq!(bad.diagnostics.len(), 1);
    assert_eq!(bad.diagnostics[0].actual.as_deref(), Some("\"deleted\""));
}

#[test]
fn signature_help_keeps_private_source_labels_and_specializes_the_checked_call() {
    let source = r#"
(ns malli-helper)
(defn- -dissoc-map-keys
  "Dissoc all keys in `keyset-map` from `from-map`."
  [from-map keyset-map]
  (reduce-kv (fn [acc k _] (dissoc acc k)) from-map keyset-map))
(defn parser-label {:t/sig '[:-> string?]} []
  (let [extras (-dissoc-map-keys {:label "kept" :omit :parser} {:omit true})]
    (:label extras)))
"#;
    let mut session = Session::default();
    let report = session.check("private.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let offset = source.find("{:omit true}").unwrap();
    let help = session.call_signatures("private.clj", offset).unwrap();
    assert_eq!(help.active_parameter, Some(1));
    assert_eq!(
        help.documentation.as_deref(),
        Some("Dissoc all keys in `keyset-map` from `from-map`.")
    );
    assert_eq!(
        help.signatures[0].parameters,
        [
            r#"from-map: {:label: "kept", :omit: :parser}"#,
            "keyset-map: {:omit: true}"
        ]
    );
    assert_eq!(
        help.signatures[0].result.as_deref(),
        Some(r#"{:label: "kept"}"#)
    );

    let changed = source.replace("{:omit true}", "{:label true}");
    let report = session.check("private.clj", &changed);
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    let help = session.call_signatures("private.clj", offset).unwrap();
    assert_eq!(
        help.signatures[0].result.as_deref(),
        Some("{:omit: :parser}")
    );
}

#[test]
fn conflicting_private_specializations_keep_the_source_only_signature() {
    let source = r#"
(ns private-context)
(defn- keep [value] value)
(defn example {:t/sig '[:-> keyword?]} []
  (let [call (fn [value] (keep value))]
    (call :first)
    (call :second)))
"#;
    let mut session = Session::default();
    let report = session.check("private.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let offset = source.find("keep value").unwrap() + "keep ".len();
    let help = session.call_signatures("private.clj", offset).unwrap();
    assert_eq!(help.signatures[0].parameters, ["value"]);
    assert!(help.signatures[0].result.is_none());
}

#[test]
fn helper_errors_point_to_body_and_explain_the_call() {
    let source = "(ns demo (:require [clojure.string :as str]))\n\
        (defn- loud [name] (str/upper-case name))\n\
        (defn label {:t/sig '[:-> string?]} [] (loud nil))";
    let report = check(source);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1);
    let error = &report.diagnostics[0];
    assert_eq!(error.actual.as_deref(), Some("Nil"));
    assert_eq!(error.span.start.line, 2);
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        "name"
    );
    assert!(
        error
            .notes
            .iter()
            .any(|n| n.contains("demo/loud") && n.contains("3:")),
        "{error:#?}"
    );
}

#[test]
fn helper_body_and_position_edits_invalidate_only_reachable_callers() {
    let source = "(ns demo)\n\
        (defn- base [x] (inc x))\n\
        (defn- wrap [x] (base x))\n\
        (defn caller {:t/sig '[number? :-> number?]} [x] (wrap x))\n\
        (defn other {:t/sig '[:-> number?]} [] 1)";
    let mut session = Session::default();
    assert!(session.check("private.clj", source).ok);
    let warm = session.check("private.clj", source);
    assert_eq!(warm.stats.checked_functions, 0);
    assert_eq!(warm.stats.reused_functions, 2);
    let changed = source.replace("(inc x)", "(str x)");
    let report = session.check("private.clj", &changed);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 1);
    assert_eq!(report.stats.reused_functions, 1);
    assert_eq!(report.diagnostics[0].actual.as_deref(), Some("String"));
    let moved = format!("\n{changed}");
    let report = session.check("private.clj", &moved);
    assert_eq!(report.diagnostics[0].span.start.line, 5);
}

#[test]
fn recursion_unused_helpers_and_first_class_uses_require_explicit_contracts() {
    for source in [
        "(defn- f [x] (f x)) (defn entry {:t/sig '[number? :-> number?]} [x] (f x))",
        "(defn- f [x] (g x)) (defn- g [x] (f x)) (defn entry {:t/sig '[number? :-> number?]} [x] (f x))",
        "(defn- unused [x] (inc x))",
        "(defn- f [x] x) (defn entry {:t/sig '[:-> number?]} [] (let [g f] (g 1)))",
        "(defn public [x] x)",
    ] {
        let report = check(source);
        assert!(!report.ok && !report.complete, "{source}: {report:#?}");
        assert!(
            report.diagnostics.iter().any(|d| d.code == "PNX9002"),
            "{report:#?}"
        );
    }
}

#[test]
fn local_bindings_and_private_predicate_names_keep_clojure_resolution() {
    let report = check(
        r#"
(ns demo (:require [clojure.spec.alpha :as s] [clojure.string :as str]))
(defn- some? [x] true)
(defn label {:t/sig '[(s/nilable string?) :-> string?]} [name]
  (if (some? name) (str/upper-case name) "Unknown"))
(defn shadow {:t/sig '[:-> string?]} [] (let [some? "local"] some?))
"#,
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Nil | String")
    );
}

#[test]
fn repeated_specializations_replay_errors_with_the_current_call_site() {
    let report = check(
        "(defn- f [x] (inc x))\n\
        (defn entry {:t/sig '[:-> number?]} []\n\
          (f \"wrong\")\n\
          (f \"wrong\")\n\
          (f 1))",
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2);
    for (error, line) in report.diagnostics.iter().zip([3, 4]) {
        assert_eq!(error.actual.as_deref(), Some("String"));
        assert_eq!(error.notes.len(), 1);
        assert!(
            error.notes[0].contains(&format!("at {line}:")),
            "{error:#?}"
        );
    }
}

#[test]
fn deep_nonrecursive_helper_chains_stop_with_an_explicit_analysis_gap() {
    let mut source = String::from("(defn entry {:t/sig '[number? :-> number?]} [x] (f0 x))\n");
    for index in 0..128 {
        source.push_str(&format!("(defn- f{index} [x] (f{} x))\n", index + 1));
    }
    source.push_str("(defn- f128 [x] x)");
    let report = check(&source);
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("inference limit"))
    );
}

#[test]
fn a_suggested_helper_fix_rechecks_and_restores_a_complete_result() {
    let source = include_str!("../examples/inferred_bad.clj");
    let mut session = Session::default();
    let report = session.check("inferred.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1);
    let suggestion = &report.diagnostics[0].suggestions[0];
    let mut fixed = source.to_string();
    fixed.replace_range(
        suggestion.span.start.offset..suggestion.span.end.offset,
        &suggestion.replacement,
    );
    let report = session.check("inferred.clj", &fixed);
    assert!(report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 1);
    assert!(check(include_str!("../examples/inferred.clj")).ok);
}

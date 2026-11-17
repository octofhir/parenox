use parenox::{Report, Session};

fn check(body: &str) -> Report {
    let source =
        format!("(ns demo (:require [clojure.spec.alpha :as s] [clojure.string :as str]))\n{body}");
    Session::default().check("updates.clj", &source)
}

#[test]
fn writes_callback_result_to_the_key_and_preserves_siblings() {
    let report = Session::default().check("updates.clj", include_str!("../examples/updates.clj"));
    assert!(report.ok && report.complete, "{report:#?}");
    for body in [
        r#"(defn f {:t/sig '[:-> number?]} [] (:x (update {:x 1} :x + 2 3)))"#,
        r#"(defn f {:t/sig '[:-> boolean?]} [] (:x (update {:x false} :x identity)))"#,
        r#"(defn f {:t/sig '[:-> nil?]} [] (get-in (update-in {:p nil} [:p :x] identity) [:p :x]))"#,
        r#"(defn f {:t/sig '[:-> number?]} [] (get-in (update-in {:p {:x 1 :name "Ada"}} [:p :name] str/trim) [:p :x]))"#,
        r#"(defn render {:t/sig '[number? string? :-> string?]} [old suffix] (str old suffix))
           (defn f {:t/sig '[:-> string?]} [] (let [updater render] (:x (update {:x 1} :x updater "!"))))"#,
        r#"(defn inc {:t/sig '[string? :-> string?]} [name] name)
           (defn f {:t/sig '[:-> string?]} [] (:x (update {:x "Ada"} :x inc)))"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
}

#[test]
fn implicit_old_arguments_get_source_diagnostics_without_edits_to_the_key() {
    let source = include_str!("../examples/updates_bad.clj");
    let report = Session::default().check("updates_bad.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2);
    for (diagnostic, key, actual) in [
        (&report.diagnostics[0], ":visits", "Nil | Number"),
        (&report.diagnostics[1], ":name", "Nil"),
    ] {
        assert_eq!(diagnostic.actual.as_deref(), Some(actual));
        assert!(diagnostic.suggestions.is_empty());
        assert_eq!(
            &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
            key
        );
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains("receives the old value"))
        );
    }
    let report = check(r#"(defn f {:t/sig '[:-> number?]} [] (:x (update {:x 1} :x + "2")))"#);
    assert_eq!(report.diagnostics[0].suggestions[0].replacement, "2");
}

#[test]
fn catches_invalid_updaters_and_untyped_paths() {
    for (body, code) in [
        ("(update {:x 1} :x inc 2)", "PNX2554"),
        ("(update-in {:x 1} [:x :y] identity)", "PNX2322"),
        ("(update-in {} [] identity)", "PNX9001"),
        ("(update-in {} [0] identity)", "PNX9001"),
        ("(update {} :x unknown-updater)", "PNX2304"),
    ] {
        let report = check(&format!("(defn f {{:t/sig '[:-> string?]}} [] {body})"));
        assert!(!report.ok, "{body}: {report:#?}");
        assert_eq!(report.diagnostics[0].code, code, "{report:#?}");
    }
    let report = check(
        r#"(defn f {:t/sig '[(s/keys) :-> string?]} [m]
             (get-in (update-in m [:x :y] str) [:x :y]))"#,
    );
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9003");
}

#[test]
fn changing_a_private_updater_invalidates_its_cached_caller() {
    let source = "(defn f {:t/sig '[:-> number?]} [] (:x (update {:x 1} :x bump)))
        (defn- bump [x] (inc x))";
    let mut session = Session::default();
    assert!(session.check("cache.clj", source).ok);
    assert_eq!(session.check("cache.clj", source).stats.reused_functions, 1);
    let report = session.check("cache.clj", &source.replace("(inc x)", "(str x)"));
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 1);
    assert_eq!(report.diagnostics[0].actual.as_deref(), Some("String"));
}

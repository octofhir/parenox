use parenox::{Report, Session};

fn check(body: &str) -> Report {
    let source = format!("(ns demo (:require [clojure.spec.alpha :as s]))\n{body}");
    Session::default().check("map_keys.clj", &source)
}

#[test]
fn checks_string_keyed_catalog_and_reports_nil_and_wrong_dictionary_values() {
    let mut session = Session::default();
    let report = session.check("map_keys.clj", include_str!("../examples/map_keys.clj"));
    assert!(report.ok && report.complete, "{report:#?}");

    let report = session.check(
        "map_keys_bad.clj",
        include_str!("../examples/map_keys_bad.clj"),
    );
    assert!(report.complete && !report.ok, "{report:#?}");
    let [lookup, update] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(lookup.actual.as_deref(), Some("Nil | Number"));
    assert_eq!(update.expected.as_deref(), Some("Map<String, Number>"));
    assert_eq!(
        update.actual.as_deref(),
        Some("Map<String, Number | String>")
    );
}

#[test]
fn scopes_dynamic_presence_to_both_bindings() {
    for (expression, expected) in [
        ("(if (contains? m k) (inc (get m k)) 0)", None),
        (
            "(if (contains? m k) (let [k other-key] (inc (get m k))) 0)",
            Some("PNX2322"),
        ),
        (
            "(if (contains? m k) (let [m other-map] (inc (get m k))) 0)",
            Some("PNX2322"),
        ),
        (
            "(if (contains? m k) (as-> other-map m (inc (get m k))) 0)",
            Some("PNX2322"),
        ),
        (
            "(if (contains? m k) (inc (get m other-key)) 0)",
            Some("PNX2322"),
        ),
        ("(if (contains? m k) 0 (req! m k))", Some("PNX2741")),
    ] {
        let report = check(&format!(
            "(s/def ::prices (s/map-of string? number?))
             (defn f {{:t/sig '[::prices string? ::prices string? :-> number?]}}
               [m k other-map other-key] {expression})"
        ));
        assert!(report.complete, "{expression}: {report:#?}");
        let codes: Vec<_> = report.diagnostics.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, expected.as_slice(), "{expression}: {report:#?}");
    }

    let report = check(
        r#"(defn f {:t/sig '[(s/map-of string? number?) string? :-> string?]} [m k]
             (if (contains? m k) "present" (get m k "missing")))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(
        "(defn f {:t/sig '[map? string? :-> number?]} [m k]
           (if (contains? m k) (get m k) 0))",
    );
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9003");
}

#[test]
fn preserves_fixed_string_fields_and_tracks_possible_dynamic_overwrites() {
    for (args, params, ret, expression, expected) in [
        ("", "[]", "string?", r#"(get {:x 1 "x" "text"} "x")"#, None),
        (
            "",
            "[]",
            "number?",
            r#"(let [{price "sku"} {"sku" 4}] (inc price))"#,
            None,
        ),
        (
            "",
            "[]",
            "(s/map-of string? number?)",
            r#"(assoc-in {"sku" 1} ["other"] 2)"#,
            None,
        ),
        (
            "",
            "[]",
            "number?",
            r#"(get-in (update-in nil ["sku" :price] (fnil inc 0)) ["sku" :price])"#,
            None,
        ),
        (
            "string?",
            "[k]",
            "number?",
            r#"(get (assoc {:price 4} k "pending") :price)"#,
            None,
        ),
        (
            "string?",
            "[k]",
            "number?",
            r#"(get (assoc {"price" 4} k "pending") "price")"#,
            Some("PNX2322"),
        ),
        ("string?", "[k]", "#{false}", "(contains? {} k)", None),
        (
            "",
            "[]",
            "number?",
            r#"(count {"x" 1 "x" 2})"#,
            Some("PNX2790"),
        ),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[{args} :-> {ret}]}} {params} {expression})"
        ));
        assert!(report.complete, "{expression}: {report:#?}");
        let codes: Vec<_> = report.diagnostics.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, expected.as_slice(), "{expression}: {report:#?}");
    }
}

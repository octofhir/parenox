use parenox::reader::{self, Kind};
use parenox::syntax::SyntaxKind;
use parenox::{Session, Source};

#[test]
fn metadata_shorthands_and_stacked_precedence_share_the_lossless_reader() {
    for (source, key) in [
        ("^:private item", ":private"),
        ("^String item", ":tag"),
        ("^\"java.lang.String\" item", ":tag"),
        ("^[String long _] item", ":param-tags"),
        ("^{:doc \"hello\"} item", ":doc"),
    ] {
        let parsed = reader::parse(source);
        assert!(parsed.errors.is_empty(), "{source}: {:?}", parsed.errors);
        assert_eq!(parsed.syntax().text().to_string(), source);
        assert!(
            parsed
                .syntax()
                .descendants()
                .any(|n| n.kind() == SyntaxKind::Metadata)
        );
        assert_eq!(parsed.forms[0].symbol(), Some("item"));
        assert!(parsed.forms[0].metadata(key).is_some());
    }
    for (source, expected) in [
        ("^:private ^{:private false} item", true),
        ("^{:private false} ^:private item", false),
    ] {
        let forms = reader::read(source).unwrap();
        assert!(
            matches!(forms[0].metadata(":private").unwrap().kind, Kind::Bool(value) if value == expected)
        );
    }
    for source in [
        "^1 item",
        "^:x 1",
        "^:x nil",
        "^:x \"text\"",
        "^:x \\a",
        "^:x",
    ] {
        let parsed = reader::parse(source);
        assert!(!parsed.errors.is_empty(), "{source}");
        assert_eq!(parsed.syntax().text().to_string(), source);
    }
}

#[test]
fn character_tokens_preserve_delimiters_named_escapes_and_jvm_character_limits() {
    let source = r"[\a \] \) \; \' \\ \space \newline \u03A9 \o141] map'";
    let parsed = reader::parse(source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_eq!(parsed.syntax().text().to_string(), source);
    let Kind::Vector(values) = &parsed.forms[0].kind else {
        panic!("Expected vector")
    };
    let actual: Vec<_> = values
        .iter()
        .map(|form| match form.kind {
            Kind::Character(c) => c,
            _ => panic!("Expected character: {form:?}"),
        })
        .collect();
    assert_eq!(
        actual,
        ['a', ']', ')', ';', '\'', '\\', ' ', '\n', 'Ω', 'a']
    );
    assert_eq!(parsed.forms[1].symbol(), Some("map'"));
    for source in [
        r"\unknown",
        r"\uD800",
        r"\u10000",
        r"\u+001",
        r"\o+1",
        r"\o400",
        "\\🚀",
        "\\",
    ] {
        let parsed = reader::parse(source);
        assert!(!parsed.errors.is_empty(), "{source}");
        assert_eq!(parsed.syntax().text().to_string(), source);
    }
}

#[test]
fn name_metadata_drives_private_inference_and_contracts_without_trusting_type_hints() {
    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s] [clojure.string :as str]))
(defn ^:private normalize [^String name] (str/upper-case (or name "Unknown")))
(defn ^{:t/sig '[(s/nilable string?) :-> string?]} label [name] (normalize name))
"#;
    let report = Session::default().check("metadata.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let report = Session::default().check(
        "hint.clj",
        "(defn f {:t/sig '[number? :-> string?]} [^String x] x)",
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].actual.as_deref(), Some("Number"));
}

#[test]
fn attribute_maps_override_name_metadata_and_private_vars_stay_private() {
    let a = r#"(ns a)
        (defn ^:private exposed {:private false :t/sig '[:-> number?]} [] 1)
        (defn ^{:private true :t/sig '[:-> number?]} hidden [] 1)"#;
    for (callee, expected_ok) in [("exposed", true), ("hidden", false)] {
        let b = format!(
            "(ns b (:require [a :as a])) (defn f {{:t/sig '[:-> number?]}} [] (a/{callee}))"
        );
        let report = Session::default().check_project(
            &[
                Source {
                    path: "a.clj",
                    source: a,
                },
                Source {
                    path: "b.clj",
                    source: &b,
                },
            ],
            None,
        );
        assert_eq!(report.ok, expected_ok, "{report:#?}");
        assert!(report.complete, "{report:#?}");
    }
}

#[test]
fn metadata_contract_edits_invalidate_cached_callers() {
    let source = "(defn ^{:t/sig '[:-> number?]} f [] 1)\n\
        (defn g {:t/sig '[:-> number?]} [] (inc (f)))";
    let mut session = Session::default();
    assert!(session.check("metadata.clj", source).ok);
    assert_eq!(
        session.check("metadata.clj", source).stats.reused_functions,
        2
    );
    let changed = source.replacen(":-> number?", ":-> string?", 1);
    let report = session.check("metadata.clj", &changed);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 2);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("clojure.core/inc"))
    );
}

#[test]
fn characters_remain_distinct_from_strings_and_obey_literal_contracts() {
    let prelude =
        r#"(ns demo (:require [clojure.spec.alpha :as s])) (s/def ::separator #{\space \-})"#;
    for (body, valid) in [
        (r"(defn f {:t/sig '[:-> char?]} [] \a)", true),
        (r"(defn f {:t/sig '[:-> string?]} [] \a)", false),
        (r"(defn f {:t/sig '[:-> ::separator]} [] \-)", true),
        (r"(defn f {:t/sig '[:-> ::separator]} [] \x)", false),
        (r#"(defn f {:t/sig '[:-> char?]} [] "a")"#, false),
    ] {
        let report = Session::default().check("chars.clj", &format!("{prelude}{body}"));
        assert_eq!(report.ok, valid, "{body}: {report:#?}");
        assert!(report.complete, "{report:#?}");
        if body.contains("string?") {
            assert_eq!(report.diagnostics[0].actual.as_deref(), Some("Character"));
        }
    }
    let report = Session::default().check(
        "guard.clj",
        r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn f {:t/sig '[(s/or :character char? :text string?) :-> char?]} [x]
          (if (char? x) x \space))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn tagged_values_are_preserved_as_data_without_running_or_guessing_reader_semantics() {
    let source =
        r#"{:id #uuid "f81d4fae-7dec-11d0-a765-00a0c91e6bf6" :value #app/value {:marker \a}}"#;
    let parsed = reader::parse(source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_eq!(parsed.syntax().text().to_string(), source);
    assert_eq!(
        parsed
            .syntax()
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::Tagged)
            .count(),
        2
    );
    let report = Session::default().check(
        "tagged.clj",
        r#"(defn f {:t/sig '[:-> string?]} [] #app/value "text")"#,
    );
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("#app/value"))
    );
    for source in ["#uuid", "#app/value", "#app//value 1", "#= (System/exit 0)"] {
        let parsed = reader::parse(source);
        assert!(!parsed.errors.is_empty(), "{source}");
        assert_eq!(parsed.syntax().text().to_string(), source);
    }
}

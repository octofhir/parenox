use parenox::{Session, Source};

const SOURCE: &str = include_str!("../examples/regex_split.cljc");

#[test]
fn regex_literal_vars_keep_their_pattern_type_across_files() {
    let provider = Source {
        path: "provider.clj",
        source: r##"(ns reader.patterns) (def jar-pattern #"^(.*\.jar):(.*)")"##,
    };
    let consumer = Source {
        path: "consumer.clj",
        source: r#"(ns reader.paths (:require [clojure.string :as str] [reader.patterns :as p]))
            (defn fields {:t/sig '[string? :-> any?]} [text]
              (str/split text p/jar-pattern))"#,
    };
    let mut session = Session::default();
    let original = session.check_project(&[consumer, provider], None);
    assert!(original.ok && original.complete, "{original:#?}");

    let edited = Source {
        source: r#"(ns reader.patterns) (def jar-pattern "not a pattern")"#,
        ..provider
    };
    let warm = session.check_project(&[consumer, edited], None);
    let cold = Session::default().check_project(&[consumer, edited], None);
    assert_eq!(
        serde_json::to_value(&warm.files[0].diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
    let [finding] = warm.files[0].diagnostics.as_slice() else {
        panic!("{warm:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.expected.as_deref(), Some("Pattern"));
    assert_eq!(
        &consumer.source[finding.span.start.offset..finding.span.end.offset],
        "p/jar-pattern"
    );
    let reverted = session.check_project(&[consumer, provider], None);
    assert!(reverted.ok && reverted.complete, "{reverted:#?}");

    let exported = session.export_externs(&[provider], &[]);
    assert!(exported.edn.is_none(), "{exported:#?}");
}

#[test]
fn required_regex_captures_exclude_nil_and_refresh_after_pattern_edits() {
    let provider = Source {
        path: "pattern.clj",
        source: r##"(ns pattern) (def value #"^(?:id:|id=)(\d+)([a-z])$")"##,
    };
    let consumer = Source {
        path: "client.clj",
        source: r#"(ns client (:require [pattern :as p] [clojure.spec.alpha :as s]))
(defn read-number {:t/sig '[string? :-> (s/nilable int?)]} [text]
  (when-let [[_ digits _] (re-find (re-pattern p/value) text)]
    (parse-long digits)))"#,
    };
    let mut session = Session::default();
    let report = session.check_project(&[consumer, provider], None);
    assert!(report.ok && report.complete, "{report:#?}");

    for pattern in [r"^(?:id:|id=)(\d+)?([a-z])$", r"^(\d+)$|^([a-z])$"] {
        let source = provider
            .source
            .replace(r"^(?:id:|id=)(\d+)([a-z])$", pattern);
        let report = session.check_project(
            &[
                consumer,
                Source {
                    source: &source,
                    ..provider
                },
            ],
            None,
        );
        let [error] = report.files[0].diagnostics.as_slice() else {
            panic!("{report:#?}")
        };
        assert_eq!(error.code, "PNX2322");
        assert_eq!(error.actual.as_deref(), Some("Nil | String"));
    }
}

#[test]
fn named_regex_capture_edits_recheck_when_the_pattern_type_stays_the_same() {
    let provider = Source {
        path: "provider.clj",
        source: r##"(ns reader.patterns) (def jar-pattern #"^(.*\.jar):(.*)")"##,
    };
    let consumer = Source {
        path: "consumer.clj",
        source: r#"(ns reader.paths (:require [reader.patterns :as p] [clojure.spec.alpha :as s]))
            (defn captures
              {:t/sig '[string? :-> (s/nilable (s/tuple string? (s/nilable string?) (s/nilable string?)))]}
              [text]
              (let [saved (re-pattern p/jar-pattern)] (re-find saved text)))
            (defn independent {:t/sig '[:-> string?]} [] "fixed")"#,
    };
    let mut session = Session::default();
    let original = session.check_project(&[consumer, provider], None);
    assert!(original.ok && original.complete, "{original:#?}");

    let edited = Source {
        source: r##"(ns reader.patterns) (def jar-pattern #"^.*\.jar:(.*)")"##,
        ..provider
    };
    let warm = session.check_project(&[consumer, edited], None);
    let cold = Session::default().check_project(&[consumer, edited], None);
    assert!(!warm.ok && warm.complete, "{warm:#?}");
    assert_eq!(warm.stats.checked_functions, 1);
    assert_eq!(warm.stats.reused_functions, 1);
    assert_eq!(
        serde_json::to_value(&warm.files[0].diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
    let [finding] = warm.files[0].diagnostics.as_slice() else {
        panic!("{warm:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        finding.actual.as_deref(),
        Some("Nil | Tuple<String, String>")
    );
    assert_eq!(
        &consumer.source[finding.span.start.offset..finding.span.end.offset],
        "(let [saved (re-pattern p/jar-pattern)] (re-find saved text))"
    );
    let reverted = session.check_project(&[consumer, provider], None);
    assert!(reverted.ok && reverted.complete, "{reverted:#?}");
}

#[test]
fn regexp_flags_preserve_capture_free_string_results() {
    let source = r##"(ns reader.uuids (:require [clojure.spec.alpha :as s]))
        (def ^:private uuid-re
          #"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
        (defn matches {:t/sig '[string? :-> (s/nilable string?)]} [text]
          (re-matches uuid-re text))
        (defn optional-prefix {:t/sig '[string? :-> (s/nilable (s/tuple string? (s/nilable string?)))]} [text]
          (re-find #"(?:a)(?=b)(b)" text))"##;
    let report = Session::default().check("matches.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn java_control_escapes_change_which_parentheses_capture() {
    let source = r##"(ns reader.regex-controls (:require [clojure.spec.alpha :as s]))
        (defn plain {:t/sig '[string? :-> (s/nilable string?)]} [text]
          (re-matches #"\c(" text))
        (defn grouped {:t/sig '[string? :-> (s/nilable (s/tuple string? (s/nilable string?)))]} [text]
          (re-matches #"\c\(a)" text))"##;
    let mut session = Session::default();
    let report = session.check("regex-controls.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let changed = source.replacen(
        "(s/nilable string?)",
        "(s/nilable (s/tuple string? (s/nilable string?)))",
        1,
    );
    let report = session.check("regex-controls.clj", &changed);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("Nil | String"));
    assert_eq!(
        &changed[finding.span.start.offset..finding.span.end.offset],
        r##"(re-matches #"\c(" text)"##
    );
}

#[test]
fn regex_matches_preserve_absence_and_optional_capture_groups() {
    let source = r##"(ns matches (:require [clojure.string :as str] [clojure.spec.alpha :as s]))
        (defn prefix {:t/sig '[string? :-> (s/nilable string?)]} [text]
          (re-find #"\A.*/" text))
        (defn groups {:t/sig '[string? :-> (s/nilable (s/tuple string? (s/nilable string?) (s/nilable string?)))]} [text]
          (re-matches #"(a)(b)?" text))
        (defn cleaned [text] (some-> (re-find #"[a-z]+" text) str/trim))"##;
    let mut session = Session::default();
    let report = session.check("matches.cljc", source);
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].severity, "warning");

    let edited = source.replace(
        "(some-> (re-find #\"[a-z]+\" text) str/trim)",
        "(str/trim (re-find #\"[a-z]+\" text))",
    );
    let report = session.check("matches.cljc", &edited);
    let mismatch = report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "PNX2322")
        .unwrap();
    assert_eq!(mismatch.actual.as_deref(), Some("Nil | String"));
    assert_eq!(
        mismatch.contract.as_ref().unwrap().name,
        "clojure.string/trim"
    );

    let matcher = r##"(ns matches)
        (defn query {:t/sig '[:-> string?]} [] (re-find (re-matcher #"a" "a")))"##;
    let report = session.check("matches.cljc", matcher);
    assert!(!report.complete);
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].severity, "warning");
}

#[test]
fn split_produces_string_vectors_and_nullable_destructuring() {
    let mut session = Session::default();
    let report = session.check("regex_split.cljc", SOURCE);
    assert!(report.ok && report.complete, "{report:#?}");

    let source = SOURCE.replace("    value))", "    (str/upper-case value)))");
    let report = session.check("regex_split.cljc", &source);
    assert!(!report.ok && report.complete, "{report:#?}");
    let mismatch = &report.diagnostics[0];
    assert_eq!(mismatch.code, "PNX2322");
    assert_eq!(mismatch.expected.as_deref(), Some("Non-nil value"));
    assert_eq!(mismatch.actual.as_deref(), Some("Nil | String"));
    assert_eq!(
        &source[mismatch.span.start.offset..mismatch.span.end.offset],
        "value"
    );

    let source = SOURCE.replace(r##"(re-pattern #"=")"##, r#""=""#);
    let report = session.check("regex_split.cljc", &source);
    let mismatch = &report.diagnostics[0];
    assert_eq!(mismatch.expected.as_deref(), Some("Pattern"));
    assert_eq!(
        mismatch.contract.as_ref().unwrap().name,
        "clojure.string/split"
    );
}

#[test]
fn pattern_contract_survives_export_and_checks_consumers() {
    let export = Session::default().export_externs(
        &[Source {
            path: "regex_split.cljc",
            source: SOURCE,
        }],
        &[],
    );
    let edn = export.edn.unwrap_or_else(|| panic!("{:#?}", export.report));
    let consumer = Source {
        path: "client.clj",
        source: r##"(ns client (:require [clojure.spec.alpha :as s]))
            (defn read-fields {:t/sig '[string? :-> (s/coll-of string? :kind vector?)]} [s]
              (demo.query-string/fields s #"="))"##,
    };
    let report = Session::default().check_project_with_externs(
        &[consumer],
        &[Source {
            path: "api.edn",
            source: &edn,
        }],
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn known_project_calls_allow_checking_string_operations_in_their_arguments() {
    let app = Source {
        path: "app.cljc",
        source: "(ns app (:require [catalog :as c] [clojure.string :as str]))\n\
            (defn label [] (c/keep (str/trim nil)))",
    };
    let catalog = Source {
        path: "catalog.clj",
        source: "(ns catalog) (defn keep [value] value)",
    };
    let mut session = Session::default();
    let plain = session.check_project(&[app, catalog], None);
    assert!(
        plain.files[0]
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX2322")
    );
    let dynamic = Source {
        source: "(ns catalog) (defn ^:dynamic keep [value] value)",
        ..catalog
    };
    let unchecked = session.check_project(&[app, dynamic], None);
    assert!(
        unchecked.files[0]
            .diagnostics
            .iter()
            .all(|finding| finding.severity == "warning")
    );
}

#[test]
fn blank_string_guards_narrow_optional_map_fields_before_replacement() {
    let mut session = Session::default();
    assert!(session.check("regex_split.cljc", SOURCE).ok);
    let edited = SOURCE.replace("(str/blank? (:value request))", "false");
    let report = session.check("regex_split.cljc", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.expected.as_deref(), Some("String"));
    assert_eq!(error.actual.as_deref(), Some("Nil | String"));
    assert_eq!(
        &edited[error.span.start.offset..error.span.end.offset],
        "(:value request)"
    );
    assert_eq!(
        error.contract.as_ref().unwrap().name,
        "clojure.string/replace"
    );
}

#[test]
fn replacement_dispatch_checks_character_and_string_pairs_and_marks_callback_gaps() {
    let source = r##"(ns replacements (:require [clojure.string :as str]))
        (defn chars {:t/sig '[:-> string?]} [] (str/replace "a-b" \- \+))
        (defn text {:t/sig '[:-> string?]} [] (str/replace "a-b" "-" "+"))
        (defn redact {:t/sig '[string? :-> string?]} [_] "x")
        (defn callback {:t/sig '[string? :-> string?]} [x] (str/replace x #"[0-9]" redact))"##;
    let mut session = Session::default();
    let report = session.check("replace.clj", source);
    let [gap] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(gap.severity, "warning");
    assert!(gap.message.contains("callback"));

    let edited = source.replace(r#"\- \+"#, r#"\- "+""#);
    let report = session.check("replace.clj", &edited);
    assert!(
        report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "PNX2322" && diagnostic.expected.as_deref() == Some("Character")
        }),
        "{report:#?}"
    );
}

#[test]
fn unannotated_code_reports_known_nil_inputs_and_keeps_type_coverage_partial() {
    let source = r##"(ns query (:require [clojure.string :as str :refer [trim]]))
        (defn field [query]
          (let [[k v] (str/split query #"=" 2)]
            (if v (str/replace v #"\+" " ") k)))
        (defn value [] (str/trim (:name {:name "Ada"})))
        (defn unknown [s] (str/trim s))
        (defn shadowed [trim] (trim nil))
        (defn nil? [_] false)
        (defn shadowed-guard [input] (when (nil? input) (str/trim input)))
        (defn opaque [] (foreign-macro (trim nil)))
        (defn quoted [] '(trim nil))"##;
    let mut session = Session::default();
    let report = session.check("query.cljc", source);
    assert!(!report.complete);
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity == "warning")
    );

    let edited = source
        .replace("(if v ", "(if true ")
        .replace(":name {:name", ":other {:name");
    let report = session.check("query.cljc", &edited);
    let mismatches: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == "PNX2322")
        .collect();
    assert_eq!(mismatches.len(), 2, "{report:#?}");
    for (diagnostic, (query, actual)) in mismatches
        .into_iter()
        .zip([("v", "Nil | String"), ("(:other {:name \"Ada\"})", "Nil")])
    {
        assert_eq!(diagnostic.actual.as_deref(), Some(actual));
        assert_eq!(diagnostic.expected.as_deref(), Some("String"));
        assert_eq!(
            &edited[diagnostic.span.start.offset..diagnostic.span.end.offset],
            query
        );
    }
    assert_eq!(
        serde_json::to_value(session.check("query.cljc", &edited).diagnostics).unwrap(),
        serde_json::to_value(report.diagnostics).unwrap()
    );
    let restored = session.check("query.cljc", source);
    assert!(
        restored
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity == "warning")
    );

    let renamed = "(ns names (:require [clojure.string :refer [trim] :rename {trim clean}]))
        (defn read-name [] (clean nil))";
    let report = session.check("query.cljc", renamed);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PNX2322")
    );
}

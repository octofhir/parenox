use parenox::reader::{self, Kind};
use parenox::syntax::SyntaxKind;
use parenox::{Session, Source};

#[test]
fn file_source_and_edn_externs_keep_their_own_literal_line_endings() {
    let source = Source {
        path: "multiline.clj",
        source: "(ns demo.multiline (:require [library :as lib]))\n\
            (def text \"a\r\nb\")\n\
            (defn source-text {:t/sig '[:-> #{\"a\\nb\"}]} [] text)\n\
            (defn extern-text {:t/sig '[:-> #{\"a\\r\\nb\"}]} [] lib/text)",
    };
    let externs = Source {
        path: "library.edn",
        source: "{library/text {:value \"a\r\nb\"}}",
    };
    let report = Session::default().check_project_with_externs(&[source], &[externs]);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn immediate_unquotes_can_duplicate_collection_keys_without_losing_syntax() {
    for (source, repeated) in [
        ("{a 1, ; ☃\n #_ :ignored `~a 2} :after", "`~a"),
        ("#{:a `~:a} :after", "`~:a"),
        ("#:demo {demo/a 1 `~a 2} :after", "`~a"),
        ("#:demo {a 1 `~demo/a 2} :after", "`~demo/a"),
        ("#:demo {:a 1 `~:demo/a 2} :after", "`~:demo/a"),
        (
            "{a 1 `(clojure.core/unquote a ignored) 2} :after",
            "`(clojure.core/unquote a ignored)",
        ),
    ] {
        let error = reader::read(source).unwrap_err();
        assert!(
            error.message.starts_with("Duplicate "),
            "{source}: {error:?}"
        );
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            repeated
        );
        let parsed = reader::parse(source);
        assert_eq!(parsed.errors.len(), 1, "{source}: {:?}", parsed.errors);
        assert_eq!(parsed.errors[0].span, error.span);
        assert_eq!(
            parsed.forms.last().and_then(|form| form.keyword()),
            Some(":after")
        );
        assert_eq!(parsed.syntax().text().to_string(), source);
    }
}

#[test]
fn projected_names_keep_their_namespace_when_checking_duplicates() {
    for source in [
        "{a 1 `a 2}",
        "#:demo {_/a 1 `~a 2}",
        "{#:demo {`~a 1} :qualified {a 1} :plain}",
    ] {
        let parsed = reader::parse(source);
        assert!(parsed.errors.is_empty(), "{source}: {:?}", parsed.errors);
        assert_eq!(parsed.syntax().text().to_string(), source);
    }

    for source in [
        "(ns reports) (def data '#::{reports/a 1 `~a 2})",
        "(ns reports) (def data '{:reports/a 1 `~::a 2})",
    ] {
        let report = Session::default().check("reports.clj", source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{source}: {report:#?}");
        };
        assert!(
            finding.message.contains("Duplicate key"),
            "{source}: {report:#?}"
        );
    }
    let distinct = "(ns reports) (def value '#:demo {_/a 1 `~a 2 ::other 3})";
    let report = Session::default().check("reports.clj", distinct);
    assert!(
        report
            .diagnostics
            .iter()
            .all(|d| !d.message.contains("Duplicate")),
        "{report:#?}"
    );
}

#[test]
fn malformed_tags_and_metadata_keep_their_operands_out_of_recovered_forms() {
    for prefix in ["#nil", "#wrong/", "^nil", "^:"] {
        for source in [
            format!("{prefix} (def phantom 1) :after"),
            format!("[{prefix}] :after"),
        ] {
            let parsed = reader::parse(&source);
            assert_eq!(parsed.syntax().text().to_string(), source);
            let [error] = parsed.errors.as_slice() else {
                panic!("{source}: {:?}", parsed.errors);
            };
            let strict = reader::read(&source).unwrap_err();
            assert_eq!(error.span, strict.span, "{source}");
            let (recovered, first) =
                reader::read_for_check_recovering(&source, reader::Conditionals::Preserve, None);
            assert_eq!(first.unwrap().span, strict.span);
            for forms in [parsed.forms, recovered.forms] {
                assert_eq!(forms.len(), 1, "{source}: {forms:?}");
                assert_eq!(forms[0].keyword(), Some(":after"));
            }
        }
    }
}

#[test]
fn recovery_keeps_nested_forms_out_of_the_file_scope() {
    let source = "(ns recovery)\n\
        (def before 1)\n\
        (let [x 1 ) (defn phantom [] :hidden)] x)\n\
        (def after before)";
    let parsed = reader::parse(source);
    assert_eq!(parsed.syntax().text().to_string(), source);
    let root_forms: Vec<_> = parsed
        .forms
        .iter()
        .map(|form| &source[form.span.start.offset..form.span.end.offset])
        .collect();
    assert_eq!(
        root_forms,
        ["(ns recovery)", "(def before 1)", "(def after before)"]
    );
    assert_eq!(parsed.errors.len(), 1);
    let error = parsed.errors[0].span;
    assert_eq!(&source[error.start.offset..error.end.offset], ")");
    assert_eq!(error.start.line, 3);

    let mut session = Session::default();
    let report = session.check("recovery.clj", source);
    assert!(!report.complete);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].span, error);
    let (_, symbols) = session.document_symbols("recovery.clj").unwrap();
    assert_eq!(
        symbols
            .into_iter()
            .map(|symbol| symbol.name)
            .collect::<Vec<_>>(),
        ["recovery", "before", "after"]
    );
}

#[test]
fn recovery_respects_prefixes_strings_and_reader_conditionals() {
    for (broken, policy) in [
        ("(do [1 '])", reader::Conditionals::Reject),
        (
            r#"(let [x "\q] (def phantom 0) "] x)"#,
            reader::Conditionals::Reject,
        ),
        (
            "(let [x 1 ) ; ] in a comment\n#_(def phantom 0) \\]] x)",
            reader::Conditionals::Reject,
        ),
        (
            "#?(42 (def phantom 0) :clj :ok)",
            reader::Conditionals::Select(":clj"),
        ),
        ("#?(def phantom 0)", reader::Conditionals::Reject),
        ("#?@(:clj [])", reader::Conditionals::Select(":clj")),
    ] {
        let source = format!("before\n{broken}\nafter");
        let parsed = reader::parse_with(&source, policy);
        let (read, error) = reader::read_for_check_recovering(&source, policy, None);
        let (_, strict) = reader::read_prefix(&source, policy);
        for forms in [&parsed.forms, &read.forms] {
            assert_eq!(
                forms.iter().map(|form| form.symbol()).collect::<Vec<_>>(),
                [Some("before"), Some("after")],
                "{broken}"
            );
        }
        assert_eq!(parsed.syntax().text().to_string(), source);
        assert_eq!(parsed.errors.len(), 1, "{broken}: {:?}", parsed.errors);
        for error in [strict.unwrap(), error.unwrap()] {
            assert_eq!(error.span, parsed.errors[0].span, "{broken}");
            assert_eq!(error.message, parsed.errors[0].message, "{broken}");
        }
    }
}

#[test]
fn references_report_type_coverage_without_a_reader_error() {
    let source = r#"
        (ns refs)
        (def state (atom 42))
        (defn read-state {:t/sig '[:-> number?]} [] @state)
        (defn var-object {:t/sig '[:-> symbol?]} [] #'read-state)
        (defn invalid-var {:t/sig '[:-> symbol?]} [] #'nil)
    "#;
    let report = Session::default().check("refs.clj", source);
    assert!(!report.complete);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|diagnostic| {
            let span = diagnostic.span;
            (
                diagnostic.code.as_str(),
                diagnostic.severity,
                &source[span.start.offset..span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX9001", "warning", "(atom 42)"),
            ("PNX9001", "warning", "@state"),
            ("PNX9001", "warning", "#'read-state"),
            ("PNX1001", "error", "#'nil"),
        ],
        "{report:#?}"
    );
}

#[test]
fn syntax_quote_leaves_macro_coverage_open_and_checks_later_functions() {
    let source = r#"
        (ns templates)
        (defmacro wrap [value options] `(vector ~value ~@options))
        (defn template {:t/sig '[:-> symbol?]} [] `tools/unknown)
        (defn checked {:t/sig '[:-> string?]} [] 42)
    "#;
    let report = Session::default().check("templates.clj", source);
    assert!(!report.complete);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.code.as_str(),
                diagnostic.severity,
                diagnostic.span.start.line,
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX9001", "warning", 3),
            ("PNX9001", "warning", 4),
            ("PNX2322", "error", 5),
        ],
        "{report:#?}"
    );
}

#[test]
fn jvm_declaration_gaps_allow_checking_a_typed_function_in_the_same_namespace() {
    let source = include_str!("../examples/jvm_forms.clj");
    let report = Session::default().check("jvm_forms.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 1);
    assert_eq!(report.diagnostics.len(), 10, "{report:#?}");
    let compiler_flags: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message.contains("compiler flag assignments"))
        .map(|diagnostic| diagnostic.span.start.line)
        .collect();
    assert_eq!(compiler_flags, [5], "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| { diagnostic.code == "PNX9001" && diagnostic.severity == "warning" }),
        "{report:#?}"
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| { !diagnostic.message.contains("has no source definition") })
    );

    let malformed = source.replace("TextBox [^String value]", "TextBox :missing-fields");
    let report = Session::default().check("jvm_forms.clj", &malformed);
    assert!(
        report.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == "error" && diagnostic.message.contains("deftype")
        }),
        "{report:#?}"
    );
}

#[test]
fn top_level_evaluation_leaves_coverage_open_for_other_declarations() {
    let source = r#"
        (ns demo (:require [clojure.test :refer [testing]]))
        (require '[clojure.string :as str])
        (let [base "hello"] (defn suffix [s] (str base s)))
        (testing "initialization" (suffix "!"))
        (set! *assert* true)
        (defn check {:t/sig '[:-> string?]} [] 1)
    "#;
    let report = Session::default().check("eval.cljc", source);
    assert!(!report.complete);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.code.as_str(),
                diagnostic.severity,
                diagnostic.span.start.line,
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX9001", "warning", 3),
            ("PNX9001", "warning", 4),
            ("PNX9001", "warning", 5),
            ("PNX9001", "warning", 6),
            ("PNX2322", "error", 7),
        ],
        "{report:#?}"
    );

    let invalid = Session::default().check("eval.cljc", "(ns demo) (let [missing]) (require)");
    assert_eq!(
        invalid
            .diagnostics
            .iter()
            .filter(|d| d.severity == "error")
            .count(),
        2
    );
}

#[test]
fn unchecked_dispatches_in_omitted_forms_leave_an_explicit_gap() {
    let source = r#"(ns reader.discarded)
#_#test/unknown :payload
(defn f {:t/sig '[:-> #{:ready}]} [] :ready)"#;
    let path = "discarded.cljc";
    let mut session = Session::default();
    let report = session.check(path, source);
    assert!(!report.complete, "{report:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
    assert!(finding.message.contains("#_"));
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "#test/unknown :payload"
    );

    let mismatch = source.replace("[] :ready", "[] :different");
    let report = session.check(path, &mismatch);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX2322")
    );
    assert!(!report.complete, "{report:#?}");

    let inactive = source.replace(
        "#_#test/unknown :payload",
        "#?(:clj (def active :ready) :cljs #_#test/unknown :payload nil)",
    );
    let report = session.check(path, &inactive);
    assert!(report.ok && report.complete, "{report:#?}");

    for regex in [
        r#"#_#"[a]""#,
        r#"#?(:clj (def active :ready) :cljs #"[a]")"#,
    ] {
        let source = source.replace("#_#test/unknown :payload", regex);
        let report = session.check(path, &source);
        assert!(!report.complete, "{report:#?}");
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, "PNX9001");
        assert!(finding.message.contains("JVM regex"));
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            r#"#"[a]""#
        );
    }
}

#[test]
fn standard_top_level_comment_removes_evaluation_but_keeps_reader_errors() {
    let source = r#"(ns comment-proof)
(comment (unloaded.ns/effect) (def unused missing-symbol) "text #a-string")
(def answer "present")
(defn result {:t/sig '[:-> #{"present"}]} [] answer)
"#;
    let path = "comment_proof.clj";
    let mut session = Session::default();
    let report = session.check(path, source);
    assert!(report.ok && report.complete, "{report:#?}");

    let shadowed = source.replace("(comment", "(def comment nil)\n(comment");
    let report = session.check(path, &shadowed);
    assert!(!report.complete, "{report:#?}");
    assert!(report.diagnostics.iter().any(|d| d.code == "PNX9001"));

    let invalid = source.replace("missing-symbol", "::unbound/item");
    let report = session.check(path, &invalid);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX9000");
    assert_eq!(
        &invalid[error.span.start.offset..error.span.end.offset],
        "::unbound/item"
    );

    for dispatch in ["#_#unknown :value", r#"#"[a]""#] {
        let opaque = source.replace("missing-symbol", dispatch);
        let report = session.check(path, &opaque);
        assert!(!report.complete, "{report:#?}");
        assert!(report.diagnostics.iter().any(|d| d.code == "PNX9001"));
    }
    let before = source.replace("(comment", "#_#unknown :value\n(comment");
    let report = session.check(path, &before);
    assert!(!report.complete, "{report:#?}");
    assert!(report.diagnostics.iter().any(|d| d.code == "PNX9001"));

    let inactive = source.replace("missing-symbol", "#?(:clj :ok :cljs #unknown :value)");
    let report = session.check("comment_proof.cljc", &inactive);
    assert!(report.ok && report.complete, "{report:#?}");

    for branches in [
        r#":clj (comment) :cljs #"[a]""#,
        r#":cljs #"[a]" :clj (comment)"#,
    ] {
        let source = format!("(ns comment-proof) #?({branches})");
        let report = session.check("comment_proof.cljc", &source);
        assert!(!report.complete, "{report:#?}");
        assert!(report.diagnostics.iter().any(|d| d.code == "PNX9001"));
    }

    let restored = session.check(path, source);
    assert!(restored.ok && restored.complete, "{restored:#?}");

    let input = Source { path, source };
    let core = Source {
        path: "clojure/core.clj",
        source: "(ns clojure.core) (def comment nil)",
    };
    let overridden = session.check_project(&[input, core], None);
    assert!(!overridden.files[0].complete, "{overridden:#?}");
    assert!(
        overridden.files[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX9001")
    );
    let restored = session.check_project(&[input], None);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn metadata_uses_immediate_syntax_quote_values() {
    assert!(matches!(
        reader::read(r#"^`"String" item"#).unwrap()[0].metadata(":tag").unwrap().kind,
        Kind::String(ref name) if name == "String"
    ));
    assert!(matches!(
        reader::read(r#"^`(clojure.core/unquote "String") item"#).unwrap()[0]
            .metadata(":tag")
            .unwrap()
            .kind,
        Kind::String(ref name) if name == "String"
    ));
    for (source, key) in [
        ("^`:private item", ":private"),
        ("^`~{:ready true} item", ":ready"),
        ("^:probe `nil", ":probe"),
        ("^:probe `true", ":probe"),
        ("^:probe ``(clojure.core/unquote)", ":probe"),
        ("^`(clojure.core/unquote :probe [:ignored]) item", ":probe"),
    ] {
        let parsed = reader::parse(source);
        assert_eq!(parsed.syntax().text().to_string(), source);
        assert!(parsed.errors.is_empty(), "{source}: {:?}", parsed.errors);
        assert!(matches!(
            parsed.forms[0].metadata(key).unwrap().kind,
            Kind::Bool(true)
        ));
    }

    for operand in [
        "`1",
        "``1",
        "`:value",
        r#"`"text""#,
        "`~nil",
        "`##Inf",
        "`(clojure.core/unquote 1)",
        "`(clojure.core/unquote 1 :ignored)",
        "`(clojure.core/unquote)",
    ] {
        let source = format!("^:probe {operand} :after");
        let strict = reader::read(&source).unwrap_err();
        let parsed = reader::parse(&source);
        assert_eq!(parsed.syntax().text().to_string(), source);
        assert_eq!(parsed.errors.len(), 1, "{source}: {:?}", parsed.errors);
        assert_eq!(parsed.errors[0].span, strict.span, "{source}");
        assert_eq!(parsed.forms.len(), 1, "{source}");
        assert_eq!(parsed.forms[0].keyword(), Some(":after"), "{source}");
    }
}

#[test]
fn ignored_explicit_unquote_operands_still_undergo_reader_validation() {
    let source = "(ns probe.metadata)\n(def ^`(clojure.core/unquote \"String\" #{::same :probe.metadata/same}) item \"value\")";
    let parsed = reader::parse(source);
    assert!(parsed.errors.is_empty(), "{parsed:#?}");
    assert_eq!(parsed.syntax().text().to_string(), source);
    let report = Session::default().check("metadata.clj", source);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX9000");
    assert!(error.message.contains("Duplicate element"));
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        ":probe.metadata/same"
    );
}

#[test]
fn syntax_quoted_metadata_keeps_unchecked_reader_dispatches_visible() {
    for (operand, message) in [
        ("#unknown/tag nil", "data reader call"),
        (r##"#"[a]""##, "regex validation"),
    ] {
        let source = format!(
            "(ns probe.metadata)\n(def ^`(clojure.core/unquote \"String\" {operand}) item \"value\")"
        );
        let parsed = reader::parse(&source);
        assert!(parsed.errors.is_empty(), "{parsed:#?}");
        assert_eq!(parsed.syntax().text().to_string(), source);
        let report = Session::default().check("metadata.clj", &source);
        assert!(!report.complete);
        let [diagnostic] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(diagnostic.code, "PNX9001");
        assert!(diagnostic.message.contains(message));
        assert!(diagnostic.message.contains("metadata descriptor"));
        assert_eq!(
            &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
            operand
        );
    }
}

#[test]
fn malformed_plain_regex_literals_fail_during_the_same_source_read() {
    for (regex, message) in [
        (r##"#"[""##, "Unclosed character class"),
        (r##"#"a)""##, "Unmatched closing group"),
        (r##"#"(a(b)""##, "Unclosed group"),
    ] {
        let source = format!(
            "(ns probe.patterns)\n(defn pattern {{:t/sig '[:-> java.util.regex.Pattern]}} [] {regex})"
        );
        let parsed = reader::parse(&source);
        assert_eq!(parsed.syntax().text().to_string(), source);
        let [error] = parsed.errors.as_slice() else {
            panic!("{parsed:#?}");
        };
        assert!(error.message.contains(message));
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            regex
        );
        let report = Session::default().check("patterns.clj", &source);
        let [diagnostic] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(diagnostic.code, "PNX9000");
        assert_eq!(diagnostic.span, error.span);
    }
    for valid in [r##"#"\Q[(\E""##, r##"#"(?x)# [""##, r##"#"[()]""##] {
        assert!(reader::read(valid).is_ok(), "{valid}");
    }
}

#[test]
fn active_metadata_tags_keep_their_unknown_value_and_inactive_tags_are_data() {
    let descriptor = r#"#probe/hint "String""#;
    let source = format!("(ns probe.metadata)\n(def ^{descriptor} item \"value\")");
    let parsed = reader::parse(&source);
    assert!(parsed.errors.is_empty(), "{parsed:#?}");
    assert_eq!(parsed.syntax().text().to_string(), source);

    let report = Session::default().check("metadata.clj", &source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete);
    assert_eq!(finding.code, "PNX9001");
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        descriptor
    );

    let data = format!("^{descriptor} item");
    assert!(reader::read_edn(&data).is_ok());
    for operand in [data.as_str(), "^:ready #probe/metadata item"] {
        let source = format!("#?(:clj 1 :cljs {operand}) :after");
        let selected = reader::parse_with(&source, reader::Conditionals::Select(":clj"));
        assert_eq!(selected.syntax().text().to_string(), source);
        assert_eq!(selected.errors.len(), 1, "{selected:#?}");
        assert!(selected.errors[0].message.contains("Metadata requires"));
        assert_eq!(selected.forms.last().unwrap().keyword(), Some(":after"));
    }
}

#[test]
fn metadata_shorthands_and_stacked_precedence_share_the_lossless_reader() {
    for (source, key) in [
        ("^:private item", ":private"),
        ("#^String item", ":tag"),
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
fn clojure_dispatch_comments_and_cr_line_endings_are_lossless() {
    let source = "#!/usr/bin/env clojure\r\n[#^String name #! skip ) ]\rnext] ; keep\rlast";
    let parsed = reader::parse(source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_eq!(parsed.syntax().text().to_string(), source);
    let comments: Vec<_> = parsed
        .syntax()
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::Comment)
        .map(|token| token.text().to_owned())
        .collect();
    assert_eq!(
        comments,
        ["#!/usr/bin/env clojure", "#! skip ) ]", "; keep"]
    );
    let bindings = parsed.forms[0].vector().unwrap();
    assert_eq!(bindings[0].symbol(), Some("name"));
    assert_eq!(
        bindings[0].metadata(":tag").unwrap().symbol(),
        Some("String")
    );
    assert_eq!(bindings[0].span.start.line, 2);
    assert_eq!(bindings[1].symbol(), Some("next"));
    assert_eq!(bindings[1].span.start.line, 3);
    assert_eq!(parsed.forms[1].symbol(), Some("last"));
    assert_eq!(parsed.forms[1].span.start.line, 4);
    assert!(reader::validate_edn("#! no EDN comment\n42").is_err());
    assert!(reader::validate_edn("; EDN comment\r42").is_ok());
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
fn edn_dispatch_checks_the_initial_tag_letter_without_losing_its_payload() {
    for source in ["#étiquette 1", "#ÿtag 1", "#vendor/λ 1"] {
        let forms = reader::read_edn(source).unwrap();
        assert!(matches!(forms[0].kind, Kind::Tagged(_, _)));
    }
    for tag in ["Ātag", "λtag", "𐐀tag"] {
        let source = format!("#{tag} (payload) :after");
        assert!(reader::read(&source).is_ok());
        let strict = reader::read_edn(&source).unwrap_err();
        let parsed = reader::parse_edn(&source);
        assert_eq!(parsed.syntax().text().to_string(), source);
        assert_eq!(parsed.errors.len(), 1, "{:?}", parsed.errors);
        assert_eq!(parsed.errors[0].span, strict.span);
        assert_eq!(parsed.forms.len(), 1);
        assert_eq!(parsed.forms[0].keyword(), Some(":after"));
    }
}

#[test]
fn edn_rejects_clojure_dispatch_forms_without_publishing_their_operands() {
    for rejected in [
        "#(vector 1)",
        r#"#"\d+""#,
        "#' (quote marker)",
        "#=(opaque)",
    ] {
        let source = format!("{rejected} :after");
        let strict = reader::read_edn(&source).unwrap_err();
        let parsed = reader::parse_edn(&source);
        assert_eq!(parsed.syntax().text().to_string(), source);
        assert_eq!(parsed.errors.len(), 1, "{rejected}: {:?}", parsed.errors);
        assert_eq!(parsed.errors[0].span, strict.span, "{rejected}");
        assert_eq!(parsed.errors[0].message, strict.message, "{rejected}");
        let error = parsed.syntax().children().next().unwrap();
        assert_eq!(error.kind(), SyntaxKind::Error, "{rejected}");
        assert_eq!(error.text().to_string(), rejected);
        assert_eq!(parsed.forms.len(), 1, "{rejected}");
        assert_eq!(parsed.forms[0].keyword(), Some(":after"), "{rejected}");
    }
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
    for (source, name) in [
        ("#-tag 1", "-tag"),
        ("# _tag 1", "_tag"),
        ("#/ 1", "/"),
        ("#app//value 1", "app//value"),
    ] {
        let parsed = reader::parse(source);
        assert!(parsed.errors.is_empty(), "{source}: {:?}", parsed.errors);
        assert_eq!(parsed.syntax().text().to_string(), source);
        assert!(
            matches!(&parsed.forms[0].kind, Kind::Tagged(tag, _) if tag.symbol() == Some(name))
        );
    }
    assert!(reader::read_edn("#-tag 1").is_err());
    assert!(reader::read_edn("# _tag 1").is_err());
    for source in ["#uuid", "#app/value", "#app/ 1", "#=(+ 1 2)"] {
        let parsed = reader::parse(source);
        assert!(!parsed.errors.is_empty(), "{source}");
        assert_eq!(parsed.syntax().text().to_string(), source);
    }
    let source = "#=(def phantom 0) :next";
    let parsed = reader::parse(source);
    assert_eq!(parsed.syntax().text().to_string(), source);
    assert_eq!(parsed.errors.len(), 1);
    assert_eq!(parsed.errors[0].kind, reader::ReadErrorKind::Unchecked);
    assert_eq!(reader::read(source).unwrap_err(), parsed.errors[0]);
    assert_eq!(parsed.forms.len(), 1);
    assert_eq!(parsed.forms[0].keyword(), Some(":next"));

    let source = "(ns demo.read-eval) (def total #=(clojure.core/+ 1 2))";
    let report = Session::default().check("read-eval.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete);
    assert_eq!(finding.code, "PNX9001");
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "#="
    );
    assert_eq!(
        reader::read_edn("#=(clojure.core/+ 1 2)").unwrap_err().kind,
        reader::ReadErrorKind::Invalid
    );
}

#[test]
fn a_data_reader_may_supply_reader_header_values() {
    for (form, tagged) in [
        ("## #probe/name Inf", "#probe/name Inf"),
        (
            "# #probe/name probe/list [:ready]",
            "#probe/name probe/list",
        ),
        ("#:#probe/name catalog{:item 7}", "#probe/name catalog"),
        ("#::#probe/name alias{:item 7}", "#probe/name alias"),
    ] {
        let source = format!("{form} :after");
        let parsed = reader::parse(&source);
        let [error] = parsed.errors.as_slice() else {
            panic!("{form}: {parsed:#?}");
        };
        assert_eq!(error.kind, reader::ReadErrorKind::Unchecked, "{form}");
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            tagged
        );
        assert_eq!(reader::read(&source).unwrap_err(), *error);
        assert_eq!(parsed.syntax().text().to_string(), source);
        assert_eq!(parsed.forms.len(), 1);
        assert_eq!(parsed.forms[0].keyword(), Some(":after"));

        let source = format!("(ns demo.headers) (def value {form})");
        let report = Session::default().check("headers.clj", &source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{form}: {report:#?}");
        };
        assert!(!report.complete);
        assert_eq!(finding.code, "PNX9001");
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            tagged
        );

        assert!(
            parsed
                .syntax()
                .descendants()
                .any(|node| { node.kind() == SyntaxKind::Tagged && node.text() == tagged })
        );
    }

    for invalid in [
        "## false",
        "#false [:ready]",
        "#:#probe/name catalog [:ready]",
        "#:#probe/name catalog{:item}",
        "#?(:clj :chosen :other ## #probe/name Inf)",
    ] {
        let error = reader::read_with(invalid, reader::Conditionals::Select(":clj")).unwrap_err();
        assert_eq!(
            error.kind,
            reader::ReadErrorKind::Invalid,
            "{invalid}: {error:#?}"
        );
    }
}

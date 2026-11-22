#[allow(dead_code)]
#[path = "support/reader_products.rs"]
mod reader;

use parenox::reader::{Conditionals, Form, Kind};
use parenox::syntax::SyntaxKind;
use parenox::{Session, Source};

fn clj(source: &str) -> Vec<Form> {
    reader::read_with(source, Conditionals::Select(":clj")).unwrap()
}

#[test]
fn syntax_preserves_all_branches_and_selection_keeps_original_spans() {
    let source = "; both platforms\n#?(:clj \"JVM\" :cljs #js {:browser true})";
    let parsed = reader::parse(source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_eq!(parsed.syntax().text().to_string(), source);
    assert!(
        parsed
            .syntax()
            .descendants()
            .any(|n| n.kind() == SyntaxKind::Conditional)
    );
    assert!(matches!(
        parsed.forms[0].kind,
        Kind::Conditional {
            splicing: false,
            ..
        }
    ));
    assert!(reader::read(source).is_err());
    let selected = reader::parse_with(source, Conditionals::Select(":clj"));
    assert!(selected.errors.is_empty(), "{:?}", selected.errors);
    assert_eq!(selected.syntax().text().to_string(), source);
    assert!(
        selected
            .syntax()
            .descendants()
            .any(|n| n.kind() == SyntaxKind::Conditional)
    );
    let forms = selected.forms;

    assert!(matches!(&forms[0].kind, Kind::String(s) if s == "JVM"));
    let span = forms[0].span;
    assert_eq!(&source[span.start.offset..span.end.offset], "\"JVM\"");
    let other = reader::read_with(source, Conditionals::Select(":cljs")).unwrap();
    assert!(matches!(other[0].kind, Kind::Tagged(_, _)));
}

#[test]
fn splicing_a_reader_prefix_uses_its_generated_list() {
    let source = "[#?@(:clj '[:ready :set]) #?@(:clj ~@items)]";
    let parsed = reader::parse_with(source, Conditionals::Select(":clj"));
    assert!(parsed.errors.is_empty(), "{parsed:#?}");
    assert_eq!(parsed.syntax().text().to_string(), source);
    for forms in [parsed.forms, clj(source)] {
        let vector = forms[0].vector().unwrap();
        let [quote, data, unquote, operand] = vector else {
            panic!("{vector:?}");
        };
        assert_eq!(quote.symbol(), Some("quote"));
        assert_eq!(data.vector().unwrap()[0].keyword(), Some(":ready"));
        assert_eq!(unquote.symbol(), Some("clojure.core/unquote-splicing"));
        assert_eq!(operand.symbol(), Some("items"));
        for (head, marker) in [(quote, "'"), (unquote, "~@")] {
            assert_eq!(
                &source[head.span.start.offset..head.span.end.offset],
                marker
            );
        }
    }
}

#[test]
fn splicing_uses_the_proven_result_of_a_syntax_quote() {
    fn items(form: &Form) -> &[Form] {
        let Kind::List(values) = &form.kind else {
            panic!("{form:#?}");
        };
        values[2].unquoted().vector().unwrap()
    }

    let policy = Conditionals::Select(":clj");
    let source = "(ns probe.splice)
        (def tokens '[0 #?@(:clj `~[:ready :set]) 3])
        (def atoms '[#?@(:clj `true) #?@(:clj `nil)])
        (def empty-template '[#?@(:clj `())])
        (def explicit '[#?@(:clj `(clojure.core/unquote [:ready] :ignored))])";
    let parsed = reader::parse_with(source, policy);
    assert!(parsed.errors.is_empty(), "{parsed:#?}");
    assert_eq!(parsed.syntax().text().to_string(), source);
    for forms in [parsed.forms, clj(source)] {
        let [_, tokens, atoms, empty, explicit] = forms.as_slice() else {
            panic!("{forms:#?}");
        };
        let tokens = items(tokens);
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[1].keyword(), Some(":ready"));
        assert_eq!(tokens[2].keyword(), Some(":set"));
        let atoms = items(atoms);
        assert_eq!(atoms.len(), 4);
        assert_eq!(atoms[0].symbol(), Some("quote"));
        let marker = atoms[0].span;
        assert_eq!(&source[marker.start.offset..marker.end.offset], "`");
        assert!(matches!(atoms[1].kind, Kind::Bool(true)));
        assert_eq!(atoms[2].symbol(), Some("quote"));
        assert!(matches!(atoms[3].kind, Kind::Nil));
        assert_eq!(items(empty)[0].symbol(), Some("clojure.core/list"));
        assert_eq!(items(explicit)[0].keyword(), Some(":ready"));
    }
    let report = Session::default().check("syntaxquote-splice.cljc", source);
    assert!(report.ok && report.complete, "{report:#?}");

    for (value, expected) in [
        ("`42", reader::ReadErrorKind::Invalid),
        ("`(clojure.core/unquote)", reader::ReadErrorKind::Invalid),
        ("`[:ready]", reader::ReadErrorKind::Unchecked),
        ("#(+ % 1)", reader::ReadErrorKind::Unchecked),
    ] {
        let source = format!("[#?@(:clj {value})] :after");
        let err = reader::read_with(&source, policy).unwrap_err();
        assert_eq!(err.kind, expected, "{source}");
        assert_eq!(&source[err.span.start.offset..err.span.end.offset], value);
        let parsed = reader::parse_with(&source, policy);
        assert_eq!(&parsed.errors, &[err]);
        assert_eq!(parsed.syntax().text().to_string(), source);
        assert_eq!(parsed.forms[0].keyword(), Some(":after"));
    }

    let source = "(ns probe.splice (:require [wire.catalog :as-alias left]
                       [wire.catalog :as-alias right]))
        (def invalid [#?@(:clj `~#{::left/item ::right/item})])";
    let report = Session::default().check("syntaxquote-splice.cljc", source);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX9000");
    assert_eq!(error.message, "Duplicate element in set literal");
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        "::right/item"
    );
}

#[test]
fn spliced_metadata_keeps_its_reader_effects_visible() {
    let source = "(ns probe.meta-splice)
        (def choices [#?@(:clj ^{:hint #reader/untracked :payload} [:ready])])";
    let mut session = Session::default();
    let path = "meta-splice.cljc";
    let report = session.check(path, source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete);
    assert_eq!(finding.code, "PNX9001");
    assert!(finding.message.contains("metadata descriptor"));
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "#reader/untracked :payload"
    );

    let edited = source.replace("#reader/untracked :payload", ":payload");
    let clean = session.check(path, &edited);
    assert!(clean.ok && clean.complete, "{clean:#?}");
    let restored = session.check(path, source);
    assert_eq!(restored.diagnostics[0].span, finding.span);
}

#[test]
fn splicing_a_data_reader_result_is_explicitly_unchecked() {
    let source = "(ns probe.tag-splice)
        (def choices [#?@(:clj #probe/list [:ready :set] :cljs [:other])])
        (def after :ok)";
    let policy = Conditionals::Select(":clj");
    let preserved = reader::parse(source);
    assert!(preserved.errors.is_empty(), "{preserved:#?}");
    assert_eq!(preserved.syntax().text().to_string(), source);
    let parsed = reader::parse_with(source, policy);
    assert_eq!(parsed.syntax().text().to_string(), source);
    let [failure] = parsed.errors.as_slice() else {
        panic!("{parsed:#?}");
    };
    assert_eq!(failure.kind, reader::ReadErrorKind::Unchecked);
    assert_eq!(reader::read_with(source, policy).unwrap_err(), *failure);
    assert_eq!(
        &source[failure.span.start.offset..failure.span.end.offset],
        "#probe/list [:ready :set]"
    );
    assert_eq!(parsed.forms.len(), 2);
    let after = parsed.forms[1].span;
    assert_eq!(
        &source[after.start.offset..after.end.offset],
        "(def after :ok)"
    );

    let mut session = Session::default();
    let path = "tag-splice.cljc";
    let report = session.check(path, source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete);
    assert_eq!(finding.code, "PNX9001");
    assert_eq!(finding.severity, "warning");
    assert_eq!(finding.span, failure.span);

    let edited = source.replace("#probe/list ", "");
    let clean = session.check(path, &edited);
    assert!(clean.ok && clean.complete, "{clean:#?}");
    let restored = session.check(path, source);
    let cold = Session::default().check(path, source);
    assert_eq!(restored.complete, cold.complete);
    assert_eq!(
        serde_json::to_value(&restored.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );

    for invalid in [
        "#?@(:clj #probe/list [:ready])",
        "[#?(:clj :chosen :cljs [#?@(:clj #probe/list [:inactive])])]",
        "[#?@(:clj #{:set})]",
        "[#?@(:clj #probe/list [:ready] :cljs (unclosed])]",
    ] {
        let error = reader::read_with(invalid, policy).unwrap_err();
        assert_eq!(error.kind, reader::ReadErrorKind::Invalid, "{invalid}");
    }
    let discarded =
        reader::read_with("#_#?@(:clj #probe/list [:ready]) :after", policy).unwrap_err();
    assert_eq!(discarded.kind, reader::ReadErrorKind::Unchecked);
}

#[test]
fn conditional_dispatch_only_allows_whitespace_before_the_list() {
    let selected = Conditionals::Select(":clj");
    let source = "#?, \n(:clj :chosen)";
    assert_eq!(clj(source)[0].keyword(), Some(":chosen"));
    assert_eq!(reader::parse(source).syntax().text().to_string(), source);
    for source in [
        "#? ;between\r\n(:clj :chosen) :after",
        "#? #_:discarded (:clj :chosen) :after",
        "#?@ #!between\n(:clj [:chosen]) :after",
    ] {
        let error = reader::read_with(source, selected).unwrap_err();
        assert_eq!(error.kind, reader::ReadErrorKind::Invalid);
        assert!(error.message.contains("requires a list"));
        for policy in [Conditionals::Preserve, selected] {
            let parsed = reader::parse_with(source, policy);
            assert_eq!(parsed.errors.as_slice(), std::slice::from_ref(&error));
            assert_eq!(parsed.syntax().text().to_string(), source);
            assert_eq!(parsed.forms.len(), 1);
            assert_eq!(parsed.forms[0].keyword(), Some(":after"));
        }
    }
}

#[test]
fn a_data_reader_can_supply_a_conditional_feature() {
    let source = "#?(#probe/feature :clj :chosen :clj :otherwise) :after";
    let policy = Conditionals::Select(":clj");
    let parsed = reader::parse_with(source, policy);
    let [error] = parsed.errors.as_slice() else {
        panic!("{parsed:#?}");
    };
    assert_eq!(error.kind, reader::ReadErrorKind::Unchecked);
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        "#probe/feature :clj"
    );
    assert_eq!(parsed.syntax().text().to_string(), source);
    assert_eq!(parsed.forms[0].keyword(), Some(":after"));
    assert_eq!(reader::read_with(source, policy).unwrap_err(), *error);

    let source = "#?(:clj :chosen :other [#?(#probe/feature :clj :ignored)])";
    let error = reader::read_with(source, policy).unwrap_err();
    assert_eq!(error.kind, reader::ReadErrorKind::Invalid);
}

#[test]
fn selection_obeys_first_match_omission_and_nested_reader_stream_semantics() {
    for (source, expected) in [
        ("#?(:cljs :browser :clj :jvm :default :fallback)", ":jvm"),
        ("#?(:default :fallback :clj :jvm)", ":fallback"),
        ("#?(:cljs :browser) :next", ":next"),
        ("#?(:other) :next", ":next"),
        ("#?(:cljs #?(:clj :nested) :clj :jvm)", ":jvm"),
        ("#?(:clj #?(:cljs :absent) :default :fallback)", ":default"),
    ] {
        let forms = clj(source);
        assert_eq!(forms.len(), 1, "{source}");
        assert_eq!(forms[0].keyword(), Some(expected), "{source}");
    }
    assert!(clj("#?(:cljs :absent)").is_empty());

    assert!(matches!(
        clj("[#?(:clj nil) #?(:cljs :absent)]")[0].vector().unwrap(),
        [Form {
            kind: Kind::Nil,
            ..
        }]
    ));
    assert!(clj("#?@(:cljs [:absent])").is_empty());
    let forms = clj("'#?(:cljs :absent) :next");
    assert_eq!(forms[0].unquoted().keyword(), Some(":next"));
    for source in [
        "#?(:cljs #?(:other :absent) :clj 2)",
        "#?(:clj 1 :cljs #?(:else 2))",
        "#?(:clj #?(:cljs 1))",
    ] {
        assert!(
            reader::read_with(source, Conditionals::Select(":clj")).is_err(),
            "{source}"
        );
    }
}

#[test]
fn deeply_nested_conditionals_preserve_text_and_stop_at_the_reader_limit() {
    let policy = Conditionals::Select(":clj");
    for (prefix, suffix) in [
        ("#?(:clj ", ")"),
        ("#?(:cljs ", " :clj :chosen)"),
        ("#?(:clj :chosen :cljs ", ")"),
    ] {
        let within_limit = 120;
        let valid = format!(
            "{}0{}",
            prefix.repeat(within_limit),
            suffix.repeat(within_limit)
        );
        assert_eq!(clj(&valid).len(), 1);

        let deep = format!("{}0{}", prefix.repeat(500), suffix.repeat(500));
        let parsed = reader::parse_with(&deep, policy);
        let [error] = parsed.errors.as_slice() else {
            panic!("{:?}", parsed.errors);
        };
        assert!(error.message.contains("nesting limit"), "{error:?}");
        assert_eq!(reader::read_with(&deep, policy).unwrap_err(), *error);
        assert_eq!(parsed.syntax().text().to_string(), deep);
    }
}

#[test]
fn splicing_inserts_list_or_vector_elements_and_validates_the_resulting_map() {
    for source in [
        "[:first #?@(:clj [:a :b] :cljs [:other]) :last]",
        "[:first #?@(:clj (:a :b)) :last]",
        "[:first #?@(:clj ^:marker [:a :b]) :last]",
    ] {
        let forms = clj(source);
        let values = forms[0].vector().unwrap();
        assert_eq!(
            values.iter().filter_map(Form::keyword).collect::<Vec<_>>(),
            [":first", ":a", ":b", ":last"]
        );
    }
    for source in [
        "{#?@(:clj [:a 1 :b 2])}",
        "#{#?@(:clj [:a :b])}",
        "(#?@(:clj [:a :b]))",
    ] {
        let parsed = reader::parse_with(source, Conditionals::Select(":clj"));
        assert!(parsed.errors.is_empty(), "{source}: {:?}", parsed.errors);
        assert_eq!(parsed.syntax().text().to_string(), source);
        assert_eq!(clj(source).len(), 1);
    }
    assert!(clj("[#?(:cljs :absent)]")[0].vector().unwrap().is_empty());
    for source in [
        "#?@(:clj [1 2])",
        "[#?@(:clj #{1 2})]",
        "[#?@(:clj nil)]",
        "{#?@(:clj [:a])}",
        "{#?(:cljs :a) 1}",
        "#?(:else 1)",
        "#?(:clj)",
        "#?(true 1)",
        "#?[1 2]",
    ] {
        assert!(
            reader::read_with(source, Conditionals::Select(":clj")).is_err(),
            "{source}"
        );
        let parsed = reader::parse(source);
        assert_eq!(parsed.syntax().text().to_string(), source, "{source}");
    }
}

#[test]
fn pending_prefix_splices_supply_entries_to_the_written_namespaced_map() {
    use reader::{Conditionals, parse_with, read_with};

    let policy = Conditionals::Select(":clj");
    let input = "[#:#?@(:clj [catalog :sku 7]) {:count 1}]";
    let parsed = parse_with(input, policy);
    assert_eq!(parsed.syntax().text().to_string(), input);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    for forms in [parsed.forms, read_with(input, policy).unwrap()] {
        let map = forms[0].vector().unwrap()[0].map().unwrap();
        assert_eq!(map.len(), 4);
        for (index, written, resolved) in
            [(0, ":sku", ":catalog/sku"), (2, ":count", ":catalog/count")]
        {
            let form = &map[index];
            assert_eq!(form.keyword(), Some(resolved));
            assert_eq!(
                &input[form.span.start.offset..form.span.end.offset],
                written
            );
        }
    }

    let malformed = "[#:#?@(:clj [catalog spare]) {:count 1}] :after";
    let parsed = parse_with(malformed, policy);
    assert_eq!(parsed.syntax().text().to_string(), malformed);
    assert_eq!(parsed.errors.len(), 1, "{:?}", parsed.errors);
    assert!(parsed.errors[0].message.contains("even number"));
    assert_eq!(parsed.forms[0].keyword(), Some(":after"));
    assert!(read_with(malformed, policy).is_err());
}

#[test]
fn spliced_values_are_read_one_at_a_time_by_prefixes_and_conditionals() {
    let policy = Conditionals::Select(":clj");
    for (source, spellings) in [
        (
            "[#_#?@(:clj [:discarded :kept]) :last]",
            vec![":kept", ":last"],
        ),
        (
            "[#_#_#?@(:clj [:one :two :three]) :last]",
            vec![":three", ":last"],
        ),
        ("[#_#?@(:clj []) :one :two]", vec![":two"]),
        (
            "[#?(:clj #?@(:clj [:selected :ignored])) :last]",
            vec![":selected", ":last"],
        ),
        (
            "[#?(:clj :selected :cljs #?@(:clj [:ignored :also-ignored])) :last]",
            vec![":selected", ":last"],
        ),
    ] {
        let parsed = reader::parse_with(source, policy);
        assert!(parsed.errors.is_empty(), "{source}: {:?}", parsed.errors);
        assert_eq!(parsed.syntax().text().to_string(), source);
        for forms in [parsed.forms, clj(source)] {
            let items = forms[0].vector().unwrap();
            assert_eq!(
                items
                    .iter()
                    .map(|item| item.keyword().unwrap())
                    .collect::<Vec<_>>(),
                spellings,
                "{source}"
            );
            for (item, expected) in items.iter().zip(&spellings) {
                assert_eq!(
                    &source[item.span.start.offset..item.span.end.offset],
                    *expected
                );
            }
        }
    }

    let source = "['#?@(:clj [:quoted :kept]) :last]";
    let forms = clj(source);
    let [quoted, kept, last] = forms[0].vector().unwrap() else {
        panic!("{forms:?}")
    };
    assert!(matches!(&quoted.kind, Kind::Quote(item) if item.keyword() == Some(":quoted")));
    assert_eq!(kept.keyword(), Some(":kept"));
    assert_eq!(last.keyword(), Some(":last"));

    let forms = clj("#_#?@(:clj [:one :two]) :after");
    assert_eq!(forms.len(), 1);
    assert_eq!(forms[0].keyword(), Some(":after"));
}

#[test]
fn cljc_checks_selected_namespaces_contracts_and_diagnostic_locations() {
    let source = include_str!("../examples/portable.cljc");
    let mut session = Session::default();
    let report = session.check("portable.cljc", source);
    assert!(report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 2);
    let report = session.check("portable.cljc", source);
    assert_eq!(report.stats.checked_functions, 0);
    assert_eq!(report.stats.reused_functions, 2);
    let changed = source.replace("(str/upper-case name)", "(str/upper-case nil)");
    let report = session.check("portable.cljc", &changed);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 1);
    let diagnostic = &report.diagnostics[0];
    assert_eq!(
        &changed[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "nil"
    );
    let report = Session::default().check("portable.clj", source);
    assert!(!report.ok && !report.complete, "{report:#?}");
}

#[test]
fn inactive_contract_edits_do_not_invalidate_other_namespaces() {
    let mut session = Session::default();
    let a = "(ns a) (defn f {:t/sig '#?(:clj [:-> number?] :cljs [:-> string?])} [] 1)";
    let b = "(ns b (:require [a :as a])) (defn g {:t/sig '[:-> number?]} [] (inc (a/f)))";
    let check = |session: &mut Session, a: &str| {
        session.check_project(
            &[
                Source {
                    path: "a.cljc",
                    source: a,
                },
                Source {
                    path: "b.clj",
                    source: b,
                },
            ],
            None,
        )
    };
    assert!(check(&mut session, a).ok);
    let inactive = a.replace("string?", "boolean?");
    let report = check(&mut session, &inactive);
    assert!(report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.reused_functions, 1);
    let changed = inactive.replace("number?", "string?");
    let report = check(&mut session, &changed);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 2);
}

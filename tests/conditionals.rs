use parenox::reader::{self, Conditionals, Form, Kind};
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
    let deep = format!("{}0{}", "#?(:clj ".repeat(1000), ")".repeat(1000));
    let parsed = reader::parse_with(&deep, Conditionals::Select(":clj"));
    assert!(
        parsed
            .errors
            .iter()
            .any(|e| e.message.contains("nesting limit"))
    );
    assert_eq!(parsed.syntax().text().to_string(), deep);
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
fn cljc_checks_selected_namespaces_contracts_and_diagnostic_locations() {
    let source = include_str!("../examples/portable.cljc");
    let mut session = Session::default();
    let report = session.check("portable.cljc", source);
    assert!(report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 2);
    let report = session.check("portable.cljc", source);
    assert_eq!(report.stats.checked_functions, 0);
    assert_eq!(report.stats.reused_functions, 2);
    let changed = source.replace("(str/upper-case name)", "(str/upper-case 4242)");
    let report = session.check("portable.cljc", &changed);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 1);
    let diagnostic = &report.diagnostics[0];
    assert_eq!(
        &changed[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "4242"
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

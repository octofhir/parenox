use parenox::reader::{self, Conditionals, Kind};
use parenox::syntax::SyntaxKind;
use parenox::{Session, Source};

#[test]
fn shorthand_arguments_preserve_reader_scope_and_concrete_syntax() {
    let source = ";; comment\n#(str #_ %4 #?(:clj % :cljs %3) %1 %&), %";
    let parsed = reader::parse_with(source, Conditionals::Select(":clj"));
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_eq!(parsed.syntax().text().to_string(), source);
    assert!(
        parsed
            .syntax()
            .descendants()
            .any(|node| node.kind() == SyntaxKind::AnonymousFunction)
    );
    let Kind::AnonymousFunction(function) = &parsed.forms[0].kind else {
        panic!("{parsed:#?}");
    };
    assert_eq!(function.required, 4);
    assert!(function.variadic);
    let aliases: Vec<_> = function
        .body
        .iter()
        .skip(1)
        .map(|form| form.symbol())
        .collect();
    assert_eq!(aliases, [Some("%1"), Some("%1"), Some("%&")]);
    assert_eq!(parsed.forms[1].symbol(), Some("%"));

    for bad in [
        "#(str %foo)",
        "#(str %01)",
        "#(str %0)",
        "#(str #_ #(str %) %)",
    ] {
        let parsed = reader::parse(bad);
        assert!(!parsed.errors.is_empty(), "{bad}");
        assert_eq!(parsed.syntax().text().to_string(), bad);
    }
}

#[test]
fn shorthand_callbacks_check_positional_rest_and_recur_arguments() {
    let good = include_str!("../examples/anonymous.clj");
    let report = Session::default().check("callbacks.clj", good);
    assert!(report.ok && report.complete, "{report:#?}");
    let bad = include_str!("../examples/anonymous_bad.clj");
    let report = Session::default().check("bad.clj", bad);
    let [element, arity] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(element.code, "PNX2322");
    assert_eq!(
        &bad[element.span.start.offset..element.span.end.offset],
        "%"
    );
    assert_eq!(arity.code, "PNX2554");
    assert!(arity.message.contains("2 arguments"));

    for (body, expected, result) in [
        (r#"(#(let [% "shadow"] %) 3)"#, "string?", None),
        (r#"(#((fn [x] (str x %2)) %1) "job/" 8)"#, "string?", None),
        (r#"(#(inc %21) 1)"#, "number?", Some("PNX2554")),
        (r#"(#(inc #_ %2 %) 1)"#, "number?", Some("PNX2554")),
        (
            r#"(#(let [{:keys [%]} {:%1 "generated"}] %) 1)"#,
            "string?",
            Some("PNX9001"),
        ),
    ] {
        let source = format!("(ns demo) (defn run {{:t/sig '[:-> {expected}]}} [] {body})");
        let report = Session::default().check("callbacks.clj", &source);
        if let Some(code) = result {
            assert_eq!(report.diagnostics.len(), 1, "{body}: {report:#?}");
            assert_eq!(report.diagnostics[0].code, code);
        } else {
            assert!(report.ok && report.complete, "{body}: {report:#?}");
        }
    }
}

#[test]
fn shorthand_bodies_track_private_helpers_and_qualified_calls_after_edits() {
    let caller = r#"(ns app (:require [rates :as rates]))
        (defn- bill [units] (rates/total units))
        (defn issue {:t/sig '[:-> number?]} [] (#(bill %) 2))"#;
    let provider = r#"(ns rates) (defn total {:t/sig '[number? :-> number?]} [n] n)"#;
    let mut session = Session::default();
    let check = |session: &mut Session, provider, caller| {
        session.check_project(
            &[
                Source {
                    path: "app.clj",
                    source: caller,
                },
                Source {
                    path: "rates.clj",
                    source: provider,
                },
            ],
            None,
        )
    };
    assert!(check(&mut session, provider, caller).ok);
    let change = provider
        .replace(":-> number?", ":-> string?")
        .replace("[n] n)", "[n] (str n))");
    let imported = check(&mut session, &change, caller);
    assert_eq!(
        imported.files[0].diagnostics[0].code, "PNX2322",
        "{imported:#?}"
    );
    assert!(check(&mut session, provider, caller).ok);
    let private = caller.replace("(rates/total units)", "(str units)");
    let edited = check(&mut session, provider, &private);
    assert_eq!(edited.files[0].stats.checked_functions, 1);
    assert_eq!(
        edited.files[0].diagnostics[0].code, "PNX2322",
        "{edited:#?}"
    );
}

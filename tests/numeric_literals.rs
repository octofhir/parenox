use parenox::Session;
use parenox::reader::{Kind, parse, read};

#[test]
fn checks_clojure_numeric_forms_through_the_regular_session() {
    let source = include_str!("../examples/numbers.clj");
    let report = Session::default().check("numbers.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    for token in [
        "+0",
        "-0XAFN",
        "010",
        "01/02",
        "00N",
        "08M",
        "1.e2",
        "1e-2M",
        "36rN",
        "1e10000000000",
        "0xFFFFFFFFFFFFFFFFFFFF",
    ] {
        let forms = read(token).unwrap();
        assert!(matches!(forms[0].kind, Kind::Number), "{token}");
        assert_eq!(forms[0].span.end.offset, token.len(), "{token}");
    }
}

#[test]
fn rejects_invalid_jvm_literals_without_losing_the_rest_of_the_cst() {
    for token in [
        "08",
        "0x",
        "37r1",
        "2r10N",
        "1/0",
        "1/00",
        "1/2M",
        "1e",
        "1.0N",
        "1e10000000000M",
        "0.00e-2147483647M",
    ] {
        let source = format!("[{token} :after]");
        let error = read(&source).unwrap_err();
        assert_eq!(error.message, "Invalid numeric literal", "{token}");
        assert_eq!(error.span.start.offset, 1, "{token}");
        assert_eq!(error.span.end.offset, 1 + token.len(), "{token}");
        assert_eq!(parse(&source).syntax().text().to_string(), source);
    }
}

#[test]
fn reader_macros_terminate_bare_numbers() {
    let source = "(1#_2 3'a 4%b)";
    let forms = read(source).unwrap();
    let Kind::List(items) = &forms[0].kind else {
        panic!("{forms:#?}");
    };
    assert_eq!(items.len(), 5);
    assert!(matches!(items[0].kind, Kind::Number));
    assert!(matches!(items[1].kind, Kind::Number));
    assert!(matches!(items[2].kind, Kind::Quote(_)));
    assert!(matches!(items[3].kind, Kind::Number));
    assert_eq!(items[4].symbol(), Some("%b"));
    assert_eq!(parse(source).syntax().text().to_string(), source);
}

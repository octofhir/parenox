use parenox_frontend::reader::{parse, read, read_edn};

#[test]
fn reader_rejects_equal_composite_keys_and_members() {
    for (source, message) in [
        ("{(1) :left [1N] :right}", "Duplicate key in map literal"),
        (
            "#{[() #{:a :b}] [[] #{:b :a}]}",
            "Duplicate element in set literal",
        ),
        (
            "#{[{:a (1) :b #{2 3}}] [{:b #{3N 2N} :a [0x1]}]}",
            "Duplicate element in set literal",
        ),
        (
            "#{#:cfg{:a [1]} {:cfg/a (1)}}",
            "Duplicate element in set literal",
        ),
        ("#{[^:a [1]] [^:b (1)]}", "Duplicate element in set literal"),
        ("#{[(quote a)] ['a]}", "Duplicate element in set literal"),
    ] {
        assert_eq!(read(source).unwrap_err().message, message, "{source}");
    }
    assert!(read_edn("#{(1) [1]}").is_err());
}

#[test]
fn reader_keeps_distinct_forms_and_skips_unsupported_composite_equality() {
    for source in [
        "#{[1 2] (2 1)}",
        "#{[1] (1.0)}",
        "#{[1] '(1)}",
        "#{[(inc 1)] [2]}",
        "#{{:a 1 :b 2} {:b 1 :a 2}}",
        "#{{##NaN :x} {##NaN :x}}",
        r##"#{{:a #"a"} {:a #"a"}}"##,
        "#{[#custom/tag 1] [#custom/tag 1]}",
    ] {
        assert!(read(source).is_ok(), "{source}");
    }
}

#[test]
fn duplicate_composite_span_and_trivia_remain_available_in_the_cst() {
    let source = "#{[1] :atom ; same composite\n  (1) :atom}\n:next";
    let parsed = parse(source);
    let [error] = parsed.errors.as_slice() else {
        panic!("{:?}", parsed.errors);
    };
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        "(1)"
    );
    assert_eq!(parsed.syntax().text().to_string(), source);
    assert_eq!(parsed.forms.last().unwrap().keyword(), Some(":next"));
}

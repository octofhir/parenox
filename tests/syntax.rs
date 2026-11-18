use parenox::reader::{self, Kind, Parse};
use parenox::syntax::SyntaxKind;

fn assert_lossless(source: &str, parsed: &Parse) {
    let root = parsed.syntax();
    assert_eq!(root.text().to_string(), source);
    let mut reconstructed = String::new();
    for token in root
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
    {
        let start: usize = token.text_range().start().into();
        let end: usize = token.text_range().end().into();
        assert_eq!(start, reconstructed.len(), "{token:?}");
        assert_eq!(&source[start..end], token.text());
        reconstructed.push_str(token.text());
    }
    assert_eq!(reconstructed, source);
}

#[test]
fn edn_data_checks_distinguish_reader_macros_and_preserve_tags_as_data() {
    for source in [
        "#:config{:result #vendor/tag ##NaN}",
        "^:m x",
        "'symbol",
        "#_ignored (quote x)",
    ] {
        reader::validate_edn(source).unwrap();
    }
    for source in [
        "[1",
        "#(inc %)",
        "#\"regex\"",
        "#?(:clj true)",
        "::name",
        "#::opts{:x 1}",
    ] {
        assert!(reader::validate_edn(source).is_err(), "{source}");
    }
}

#[test]
fn cst_retains_comments_commas_crlf_unicode_and_discarded_forms() {
    let source =
        " ; лиса\r\n#_ #_ :ignored :also-ignored\r\n{:name, \"лиса\\u0021\", :tags #{:active}}\n";
    let parsed = reader::parse(source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_lossless(source, &parsed);
    assert_eq!(
        parsed
            .syntax()
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::Discard)
            .count(),
        2
    );
    let kinds: Vec<_> = parsed
        .syntax()
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .map(|token| token.kind())
        .collect();
    for kind in [
        SyntaxKind::Comment,
        SyntaxKind::Comma,
        SyntaxKind::Whitespace,
        SyntaxKind::OpenSet,
    ] {
        assert!(kinds.contains(&kind), "{kind:?}");
    }
    assert_eq!(parsed.forms.len(), 1);
    let Kind::Map(fields) = &parsed.forms[0].kind else {
        panic!("Expected map")
    };
    assert!(matches!(&fields[1].kind, Kind::String(text) if text == "лиса!"));
    assert!(matches!(fields[3].kind, Kind::Set(_)));
}

#[test]
fn mismatched_delimiters_keep_later_forms_and_all_original_tokens() {
    let source = "(broken [1)\n(defn later {:t/sig '[:-> number?]} [] 42)\n]";
    let parsed = reader::parse(source);
    assert_eq!(parsed.errors.len(), 2, "{:?}", parsed.errors);
    assert_lossless(source, &parsed);
    assert!(
        parsed
            .syntax()
            .descendants()
            .any(|node| node.kind() == SyntaxKind::Error)
    );
    assert_eq!(parsed.forms.len(), 1);
    let Kind::List(items) = &parsed.forms[0].kind else {
        panic!("Expected later defn")
    };
    assert_eq!(items[1].symbol(), Some("later"));
    // Checking remains strict: partial syntax recovery does not imply a complete typecheck.
    let report = parenox::Session::default().check("broken.clj", source);
    assert!(!report.ok && !report.complete);
}

#[test]
fn regex_literals_preserve_pattern_escapes_instead_of_decoding_strings() {
    let source = r##"#"\A[\d\p{L}]+\"\z" :after"##;
    let parsed = reader::parse(source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_lossless(source, &parsed);
    assert!(matches!(parsed.forms[0].kind, Kind::Regex));
    assert_eq!(parsed.forms[1].keyword(), Some(":after"));
    assert!(
        parsed
            .syntax()
            .descendants_with_tokens()
            .any(|element| element.kind() == SyntaxKind::Regex)
    );
    assert!(reader::read(r##"#"pattern\"##).is_err());
}

#[test]
fn malformed_and_unsupported_reader_syntax_is_lossless_and_terminates() {
    for source in [
        "#_",
        "#_ #_",
        "' #_",
        "'",
        "{1}",
        "\"\\q\"",
        "\"unfinished",
        "#?(:clj :x",
        r"\unknown",
        "1/0",
    ] {
        let parsed = reader::parse(source);
        assert!(!parsed.errors.is_empty(), "{source:?}");
        assert_lossless(source, &parsed);
        let first = reader::read_with(source, reader::Conditionals::Preserve).unwrap_err();
        assert_eq!(parsed.errors[0].message, first.message);
        assert_eq!(parsed.errors[0].span, first.span);
    }
}

#[test]
fn quotes_and_trivia_only_inputs_keep_exact_source() {
    for source in ["", " , ; comment\r\n\t", "' ; before the data\n [1, #{:x}]"] {
        let parsed = reader::parse(source);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        assert_lossless(source, &parsed);
    }
    let parsed = reader::parse("' ; comment\n[1]");
    let quote = parsed.syntax().children().next().unwrap();
    assert_eq!(quote.kind(), SyntaxKind::Quote);
    assert_eq!(quote.children().next().unwrap().kind(), SyntaxKind::Vector);
}

#[test]
fn nesting_limit_preserves_the_unread_tail_without_overflow() {
    let source = format!("{}0{}", "(".repeat(1000), ")".repeat(1000));
    let parsed = reader::parse(&source);
    assert!(
        parsed
            .errors
            .iter()
            .any(|error| error.message.contains("nesting limit"))
    );
    assert_lossless(&source, &parsed);
}

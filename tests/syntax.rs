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
fn string_escapes_preserve_jvm_utf16_values_and_original_text() {
    let source = r#"["\uD83D\uDE03!" "a\uD800😃\n"]"#;
    let parsed = reader::parse(source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_lossless(source, &parsed);
    let Some([paired, unpaired]) = parsed.forms[0].vector() else {
        panic!("{:?}", parsed.forms);
    };
    assert!(matches!(&paired.kind, Kind::String(text) if text == "😃!"));
    assert!(matches!(&unpaired.kind, Kind::Utf16String(units)
        if units == &[0x0061, 0xd800, 0xd83d, 0xde03, 0x000a]));
    assert!(reader::validate_edn(source).is_ok());
    assert!(reader::validate_edn(r#""\u12x4""#).is_err());
    assert!(reader::read(r#"#{"\uD800" "\ud800"}"#).is_err());
    assert!(reader::read(r#"#{"\uD83D\uDE03" "😃"}"#).is_err());
}

#[test]
fn symbolic_numbers_use_the_same_reader_in_code_and_edn() {
    let source = "[##Inf ##-Inf ## ; comment\n #_discard NaN]";
    let parsed = reader::parse(source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_lossless(source, &parsed);
    assert!(reader::validate_edn(source).is_ok());
    let numbers = parsed.forms[0]
        .vector()
        .unwrap()
        .iter()
        .map(|form| match form.kind {
            Kind::SymbolicNumber(number) => number.spelling(),
            _ => panic!("{form:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(numbers, ["##Inf", "##-Inf", "##NaN"]);
    for invalid in ["##+Inf", "##NaN#extra", "##[NaN]", "#{##NaN ## NaN}"] {
        assert!(reader::read(invalid).is_err(), "{invalid}");
        assert!(reader::validate_edn(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn syntax_quote_preserves_templates_and_embedded_forms_without_expanding_them() {
    let source = "`(let [x# ~ ; note\n value] `[x# ~@items]) :after";
    let parsed = reader::parse(source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_lossless(source, &parsed);
    let Kind::SyntaxQuote(template) = &parsed.forms[0].kind else {
        panic!("{:?}", parsed.forms);
    };
    let Kind::List(parts) = &template.kind else {
        panic!("{template:?}");
    };
    let Some([name, value]) = parts[1].vector() else {
        panic!("{parts:?}");
    };
    assert_eq!(name.symbol(), Some("x#"));
    assert!(
        matches!(&value.kind, Kind::Unquote { splicing: false, form }
        if form.symbol() == Some("value"))
    );
    let Kind::SyntaxQuote(nested) = &parts[2].kind else {
        panic!("{parts:?}");
    };
    assert!(
        matches!(&nested.vector().unwrap()[1].kind, Kind::Unquote { splicing: true, form }
        if form.symbol() == Some("items"))
    );
    assert_eq!(parsed.forms[1].keyword(), Some(":after"));
    let nodes: Vec<_> = parsed
        .syntax()
        .descendants()
        .map(|node| node.kind())
        .collect();
    assert!(nodes.contains(&SyntaxKind::UnquoteSplicing));
    assert!(reader::read("`~@items").is_err());
    assert!(reader::read("`(foo ~)").is_err());
    assert!(reader::validate_edn(source).is_err());
}

#[test]
fn deref_and_var_quote_keep_source_and_expand_to_jvm_reader_forms() {
    let source = "@ #_ignored app/state\n#' ; reference\n app/read-state";
    let parsed = reader::parse(source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_lossless(source, &parsed);
    let syntax = parsed.syntax();
    let nodes: Vec<_> = syntax.children().map(|node| node.kind()).collect();
    assert_eq!(nodes, [SyntaxKind::Deref, SyntaxKind::VarQuote]);
    for (form, (operator, operand, spelling)) in reader::read(source).unwrap().iter().zip([
        ("clojure.core/deref", "app/state", "@"),
        ("var", "app/read-state", "#'"),
    ]) {
        let Kind::List(items) = &form.kind else {
            panic!("{form:?}");
        };
        let [head, argument] = items.as_slice() else {
            panic!("{items:?}");
        };
        assert_eq!(head.symbol(), Some(operator));
        assert_eq!(argument.symbol(), Some(operand));
        assert_eq!(
            &source[head.span.start.offset..head.span.end.offset],
            spelling
        );
        assert_eq!(
            &source[argument.span.start.offset..argument.span.end.offset],
            operand
        );
    }
    for unsupported in ["@x", "#'x"] {
        assert!(reader::validate_edn(unsupported).is_err());
    }
}

#[test]
fn duplicate_atomic_set_members_point_to_the_repeated_value() {
    let source = r#"#{:found :found} :later"#;
    let parsed = reader::parse(source);
    assert_lossless(source, &parsed);
    assert_eq!(parsed.forms[0].keyword(), Some(":later"));
    let error = &parsed.errors[0];
    assert_eq!(error.message, "Duplicate element in set literal");
    assert_eq!(error.span.start.offset, 9);
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        ":found"
    );

    assert!(reader::validate_edn(r#"#{ "name" "n\u0061me" }"#).is_err());
    assert!(reader::read(r##"#{nil false :name "name" name #"x" #"x" 1 1.0}"##).is_ok());
    let selected = reader::read_with(
        "#{:found #?@(:clj [:found] :cljs [:missing])}",
        reader::Conditionals::Select(":clj"),
    );
    assert!(selected.is_err());
    assert!(
        reader::read_with(
            "#?(:clj 1 :cljs #{:a :a})",
            reader::Conditionals::Select(":clj")
        )
        .is_err()
    );
}

#[test]
fn duplicate_map_keys_report_the_second_spelling_and_preserve_recovery() {
    let source = r#"{:enabled true "name" 1 "n\u0061me" 2} :after"#;
    let parsed = reader::parse(source);
    assert_lossless(source, &parsed);
    let [error] = parsed.errors.as_slice() else {
        panic!("{:?}", parsed.errors);
    };
    assert_eq!(error.message, "Duplicate key in map literal");
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        r#""n\u0061me""#
    );
    assert_eq!(parsed.forms[0].keyword(), Some(":after"));
    assert!(reader::validate_edn(source).is_err());
    assert!(reader::read(r#"{:name 1 "name" 2 "other" 2}"#).is_ok());
    assert!(
        reader::read_with(
            "{:name 1 #?@(:clj [:name 2] :cljs [:other 3])}",
            reader::Conditionals::Select(":clj"),
        )
        .is_err()
    );
}

#[test]
fn explicit_namespace_maps_qualify_keys_in_both_readers_and_keep_spelling() {
    let source = r#"#:stock {:sku "id" :_/category :input :vendor/price 1 code 7 _/raw 9}"#;
    let parsed = reader::parse(source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_lossless(source, &parsed);
    assert_eq!(
        parsed.syntax().children().next().unwrap().kind(),
        SyntaxKind::NamespacedMap
    );
    for forms in [parsed.forms, reader::read_edn(source).unwrap()] {
        let entries = forms[0].map().unwrap();
        let keys: Vec<_> = entries
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[key, _]| key.keyword().or_else(|| key.symbol()).unwrap())
            .collect();
        assert_eq!(
            keys,
            [
                ":stock/sku",
                ":category",
                ":vendor/price",
                "stock/code",
                "raw"
            ]
        );
        assert_eq!(
            &source[entries[0].span.start.offset..entries[0].span.end.offset],
            ":sku"
        );
    }
    for read in [reader::read, reader::read_edn] {
        assert!(read("#:stock{:sku 1 :stock/sku 2}").is_err());
        assert!(read("#:stock/item{:sku 1}").is_err());
        assert!(read("#:stock ; comment\n{:sku 1}").is_err());
    }
    assert!(reader::read_edn("#: stock {:sku 1}").is_ok());
    assert!(reader::read("#: stock {:sku 1}").is_err());
    let selected = reader::read_with(
        "#:stock{#?@(:clj [:sku 1] :cljs [:other 2])}",
        reader::Conditionals::Select(":clj"),
    )
    .unwrap();
    assert_eq!(selected[0].map().unwrap()[0].keyword(), Some(":stock/sku"));
}

#[test]
fn auto_namespaced_maps_keep_the_raw_prefix_until_source_context_exists() {
    let source = "#::payload {:value 1} #:: {:value 2}";
    let parsed = reader::parse(source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_lossless(source, &parsed);
    assert!(
        matches!(&parsed.forms[0].kind, Kind::AutoNamespacedMap { alias: Some(alias), entries }
        if alias == "payload" && entries[0].keyword() == Some(":value"))
    );
    assert!(matches!(
        &parsed.forms[1].kind,
        Kind::AutoNamespacedMap { alias: None, .. }
    ));
    assert!(reader::validate_edn(source).is_err());
    assert!(reader::read("#:: payload {:value 1}").is_err());
}

#[test]
fn malformed_and_unsupported_reader_syntax_is_lossless_and_terminates() {
    for source in [
        "#_",
        "#_ #_",
        "' #_",
        "'",
        "@",
        "#'",
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

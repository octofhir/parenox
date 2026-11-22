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
fn symbol_keyword_and_numeric_token_grammar_matches_the_jvm() {
    for token in [
        ":",
        ":::foo",
        ":foo:",
        "a::b",
        "λ::x",
        "foo/",
        "/foo",
        "foo/0",
        ":foo/2",
        "١",
        "+١",
        "a\u{85}b/name",
    ] {
        let source = format!("{token} :after");
        let parsed = reader::parse(&source);
        assert_lossless(&source, &parsed);
        let [error] = parsed.errors.as_slice() else {
            panic!("{token}: {:?}", parsed.errors)
        };
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            token
        );
        assert_eq!(parsed.forms[0].keyword(), Some(":after"));
        assert!(reader::read_edn(token).is_err(), "{token}");
    }
    for spelling in ["/", "foo//", "foo/bar/baz", "foo/:bar", "²", "Ⅳ", "𝟐", "+𝟐"] {
        let forms = reader::read(spelling).unwrap();
        assert_eq!(forms[0].symbol(), Some(spelling));
    }
    for spelling in [":2", "::2", ":foo//", ":/"] {
        let forms = reader::read(spelling).unwrap();
        assert_eq!(forms[0].keyword(), Some(spelling));
    }
    for spelling in ["long/1", "String/2", "java.lang.String/9", "a:/2"] {
        let forms = reader::read(spelling).unwrap();
        assert_eq!(forms[0].symbol(), Some(spelling));
        assert!(reader::read_edn(spelling).is_err(), "{spelling}");
    }
}

#[test]
fn unfinished_names_keep_their_token_kind_without_becoming_forms() {
    let source = "(f {: 0}) (alias/) :ok";
    let parsed = reader::parse(source);
    assert_lossless(source, &parsed);
    assert_eq!(parsed.errors.len(), 2);
    assert_eq!(parsed.forms.len(), 1);
    assert_eq!(parsed.forms[0].keyword(), Some(":ok"));
    let syntax = parsed.syntax();
    let token = |spelling| {
        syntax
            .descendants_with_tokens()
            .filter_map(|element| element.into_token())
            .find(|token| token.text() == spelling)
            .unwrap()
    };
    for (spelling, kind) in [(":", SyntaxKind::Keyword), ("alias/", SyntaxKind::Symbol)] {
        let token = token(spelling);
        assert_eq!(token.kind(), kind);
        assert_eq!(token.parent().unwrap().kind(), SyntaxKind::Error);
    }
}

#[test]
fn whitespace_and_token_boundaries_follow_the_jvm_readers() {
    for (separator, is_whitespace) in [
        ('\u{001c}', true),
        ('\u{001f}', true),
        ('\u{1680}', true),
        ('\u{2008}', true),
        ('\u{3000}', true),
        ('\u{0085}', false),
        ('\u{00a0}', false),
        ('\u{2007}', false),
        ('\u{202f}', false),
    ] {
        let token = format!("left{separator}right");
        let source = format!("[{token}] #:example{separator}{{:key 1}}");
        let parsed = reader::parse(&source);
        assert_lossless(&source, &parsed);
        for forms in [
            parsed.forms,
            reader::read(&source).unwrap(),
            reader::read_edn(&source).unwrap(),
        ] {
            let items = forms[0].vector().unwrap();
            let names: Vec<_> = items.iter().map(|item| item.symbol().unwrap()).collect();
            let prefix = if is_whitespace {
                "example".into()
            } else {
                format!("example{separator}")
            };
            assert_eq!(
                names,
                if is_whitespace {
                    vec!["left", "right"]
                } else {
                    vec![token.as_str()]
                }
            );
            assert_eq!(
                forms[1].map().unwrap()[0].keyword(),
                Some(format!(":{prefix}/key").as_str())
            );
            assert_eq!(
                &source[items[0].span.start.offset..items[0].span.end.offset],
                names[0]
            );
        }

        let character = format!("[\\a{separator}:right]");
        let octal = format!("\"\\1{separator}x\"");
        for read in [reader::read, reader::read_edn] {
            if is_whitespace {
                let forms = read(&character).unwrap();
                assert!(matches!(
                    forms[0].vector().unwrap()[0].kind,
                    Kind::Character('a')
                ));
                let forms = read(&octal).unwrap();
                assert!(
                    matches!(&forms[0].kind, Kind::String(text) if text == &format!("\u{1}{separator}x"))
                );
            } else {
                assert!(read(&character).is_err());
                assert!(read(&octal).is_err());
            }
        }
    }
}

#[test]
fn mismatched_delimiters_keep_later_forms_and_all_original_tokens() {
    let source = "(broken [1)])\n(defn later {:t/sig '[:-> number?]} [] 42)\n]";
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
fn an_ambiguous_closing_delimiter_does_not_promote_a_nested_form() {
    let source = "(broken [1)\n(defn maybe-nested [] 42)\n]";
    let parsed = reader::parse(source);
    assert_lossless(source, &parsed);
    assert!(parsed.forms.is_empty());
    assert_eq!(parsed.errors.len(), 1);
    let error = &parsed.errors[0].span;
    assert_eq!(&source[error.start.offset..error.end.offset], ")");
}

#[test]
fn regex_literals_preserve_pattern_escapes_instead_of_decoding_strings() {
    let source = r##"#"\A[\d\p{L}]+\"\z" :after"##;
    let parsed = reader::parse(source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_lossless(source, &parsed);
    assert!(matches!(
        parsed.forms[0].kind,
        Kind::Regex { captures: Some(0) }
    ));
    assert_eq!(parsed.forms[1].keyword(), Some(":after"));
    assert!(
        parsed
            .syntax()
            .descendants_with_tokens()
            .any(|element| element.kind() == SyntaxKind::Regex)
    );
    assert!(reader::read(r##"#"pattern\"##).is_err());

    for (source, count) in [
        (r##"#"(a)(b)?""##, Some(2)),
        (r##"#"\(""##, Some(0)),
        (r##"#"[()]""##, None),
        (r##"#"(?i)(a)""##, Some(1)),
        (r##"#"(?i)[a-f]+""##, Some(0)),
        (r##"#"(?<=a)(b)""##, Some(1)),
        (r##"#"(?x)(a)""##, None),
        (r##"#"\c(""##, Some(0)),
        (r##"#"\c\(a)""##, Some(1)),
    ] {
        let parse = reader::parse(source);
        assert!(parse.errors.is_empty(), "{source}: {:?}", parse.errors);
        assert_lossless(source, &parse);
        let semantic = reader::read(source).unwrap();
        for form in [&parse.forms[0], &semantic[0]] {
            assert!(matches!(form.kind, Kind::Regex { captures } if captures == count));
        }
    }
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
fn ascii_string_prefixes_preserve_following_values_and_positions() {
    let prefix = "a".repeat(64);
    let source = format!("[\"{prefix}\\uD83D\\uDE03\" \"{prefix}\r\n😃\" :after]");
    let parsed = reader::parse(&source);
    assert_lossless(&source, &parsed);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    for forms in [parsed.forms, reader::read(&source).unwrap()] {
        let values = forms[0].vector().unwrap();
        assert!(matches!(&values[0].kind, Kind::String(text) if text == &format!("{prefix}😃")));
        assert!(matches!(&values[1].kind, Kind::String(text) if text == &format!("{prefix}\n😃")));
        assert_eq!(values[2].keyword(), Some(":after"));
        let start = values[2].span.start;
        assert_eq!(start.offset, source.find(":after").unwrap());
        assert_eq!((start.line, start.column), (2, 4));
    }
}

#[test]
fn source_literal_line_endings_are_normalized_without_changing_spans() {
    let source = "[\"a\r\nb\rc\" \"a\\r\\nb\\rc\" \\\r\n \\\r :after]";
    let parsed = reader::parse(source);
    assert_lossless(source, &parsed);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    for forms in [parsed.forms, reader::read(source).unwrap()] {
        let vector = forms[0].vector().unwrap();
        let [physical, escaped, crlf, cr, after] = vector else {
            panic!("{vector:?}");
        };
        assert!(matches!(&physical.kind, Kind::String(text) if text == "a\nb\nc"));
        assert!(matches!(&escaped.kind, Kind::String(text) if text == "a\r\nb\rc"));
        assert!(matches!(crlf.kind, Kind::Character('\n')));
        assert!(matches!(cr.kind, Kind::Character('\n')));
        assert_eq!(
            &source[crlf.span.start.offset..crlf.span.end.offset],
            "\\\r\n"
        );
        assert_eq!(after.span.start.offset, source.find(":after").unwrap());
        assert_eq!((after.span.start.line, after.span.start.column), (5, 2));
    }

    let edn = reader::parse_edn(source);
    assert_lossless(source, &edn);
    let vector = edn.forms[0].vector().unwrap();
    assert!(matches!(&vector[0].kind, Kind::String(text) if text == "a\r\nb\rc"));
    assert!(matches!(vector[2].kind, Kind::Character('\r')));
    let data = reader::read_edn(source).unwrap();
    assert!(
        matches!(&data[0].vector().unwrap()[0].kind, Kind::String(text) if text == "a\r\nb\rc")
    );
    assert!(reader::read("#{\"a\r\nb\" \"a\\nb\"}").is_err());
    assert!(reader::read_edn("#{\"a\r\nb\" \"a\\nb\"}").is_ok());
}

#[test]
fn unicode_digits_in_escapes_follow_java_character_digit() {
    let source = r#"[\uＦＦＦＦ \u१२３４ \o٧٧ "\uＦＦＦＦ\u१२３４\٧٧" "\uＤ８３Ｄ\uＤＥ０３"]"#;
    let parsed = reader::parse(source);
    assert_lossless(source, &parsed);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    for forms in [parsed.forms, reader::read_edn(source).unwrap()] {
        let values = forms[0].vector().unwrap();
        assert!(matches!(values[0].kind, Kind::Character('\u{ffff}')));
        assert!(matches!(values[1].kind, Kind::Character('\u{1234}')));
        assert!(matches!(values[2].kind, Kind::Character('?')));
        assert!(matches!(&values[3].kind, Kind::String(value) if value == "\u{ffff}\u{1234}?"));
        assert!(matches!(&values[4].kind, Kind::String(value) if value == "😃"));
    }

    for token in [
        r"\uＤ８００",
        r"\u𝟏234",
        r"\o٤٠٠",
        r"\o８",
        r#""\uＦＦＧＦ""#,
        r#""\٧٨""#,
    ] {
        let source = format!("{token} :after");
        for parse in [reader::parse, reader::parse_edn] {
            let parsed = parse(&source);
            assert_lossless(&source, &parsed);
            assert_eq!(parsed.errors.len(), 1, "{token}");
            assert_eq!(parsed.errors[0].span.start.offset, 0, "{token}");
            assert_eq!(parsed.forms[0].keyword(), Some(":after"), "{token}");
        }
    }
}

#[test]
fn octal_string_escapes_match_clojure_and_edn_token_boundaries() {
    let source = r#"["\033[38;5;" "\377" "\1;"]"#;
    let parsed = reader::parse(source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_lossless(source, &parsed);
    for forms in [parsed.forms, reader::read_edn(source).unwrap()] {
        let values = forms[0].vector().unwrap();
        for (form, expected) in values.iter().zip(["\u{1b}[38;5;", "\u{ff}", "\u{1};"]) {
            assert!(matches!(&form.kind, Kind::String(text) if text == expected));
        }
    }
    for bad in [r#""\400""#, r#""\08""#, r#""\1a""#] {
        assert!(reader::read(bad).is_err(), "{bad}");
        assert!(reader::read_edn(bad).is_err(), "{bad}");
    }
    let reader_quote_mark = r#""\1'abc""#;
    assert!(reader::read(reader_quote_mark).is_ok());
    assert!(reader::read_edn(reader_quote_mark).is_err());
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
    assert!(reader::read("`(clojure.core/unquote-splicing items)").is_err());
    assert!(reader::read("`(list (clojure.core/unquote-splicing items))").is_ok());
    assert!(reader::read("`(foo ~)").is_err());
    assert!(reader::validate_edn(source).is_err());
}

#[test]
fn surrounding_syntax_quote_validates_the_result_of_inner_unquote() {
    for template in [
        "``~~@items",
        "``~(clojure.core/unquote-splicing items)",
        "``(clojure.core/unquote (clojure.core/unquote-splicing items))",
    ] {
        let source = format!("{template} :after");
        let strict = reader::read(&source).unwrap_err();
        let parsed = reader::parse(&source);
        assert_lossless(&source, &parsed);
        let [error] = parsed.errors.as_slice() else {
            panic!("{parsed:#?}");
        };
        assert_eq!(error.span, strict.span);
        assert!(error.message.contains("surrounding collection"));
        assert_eq!(parsed.forms.len(), 1);
        assert_eq!(parsed.forms[0].keyword(), Some(":after"));
    }

    for valid in ["`~(clojure.core/unquote-splicing items)", "`[`~~@items]"] {
        assert!(reader::read(valid).is_ok(), "{valid}");
    }
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
fn literal_duplicates_compare_unquote_markers_with_their_reader_lists() {
    for (source, duplicate) in [
        (
            "'#{~x (clojure.core/unquote x)} :after",
            "(clojure.core/unquote x)",
        ),
        (
            "'#{~@items [clojure.core/unquote-splicing items]} :after",
            "[clojure.core/unquote-splicing items]",
        ),
        ("'{~x :first ~x :second} :after", "~x"),
    ] {
        let parsed = reader::parse(source);
        assert_lossless(source, &parsed);
        let [error] = parsed.errors.as_slice() else {
            panic!("{parsed:#?}");
        };
        assert!(error.message.starts_with("Duplicate "));
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            duplicate
        );
        assert_eq!(reader::read(source).unwrap_err().span, error.span);
        assert_eq!(parsed.forms.len(), 1);
        assert_eq!(parsed.forms[0].keyword(), Some(":after"));
    }
    assert!(reader::read("'#{~x ~@x (unquote x)}").is_ok());
}

#[test]
fn literal_duplicates_compare_already_read_syntax_quote_atoms() {
    for (literal, duplicate) in [
        ("#{3 `3}", "`3"),
        (r##"#{"sample" `"sample"}"##, r#"`"sample""#),
        (r"#{\A `\A}", r"`\A"),
        (
            "{nil :first `(clojure.core/unquote) :second}",
            "`(clojure.core/unquote)",
        ),
    ] {
        let source = format!("{literal} :after");
        let parsed = reader::parse(&source);
        assert_lossless(&source, &parsed);
        let [error] = parsed.errors.as_slice() else {
            panic!("{parsed:#?}");
        };
        assert!(error.message.starts_with("Duplicate "));
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            duplicate
        );
        assert_eq!(reader::read(&source).unwrap_err().span, error.span);
        assert_eq!(parsed.forms[0].keyword(), Some(":after"));
    }
    for valid in ["#{true `true}", "#:qualified{`:label 1 :_/label 2}"] {
        assert!(reader::read(valid).is_ok(), "{valid}");
    }
}

#[test]
fn literal_uniqueness_compares_integer_values_across_reader_spellings() {
    for (source, duplicate) in [
        ("#{1 01}", "01"),
        ("#{-0N 0}", "0"),
        ("#{0xffN 16rFF}", "16rFF"),
        ("#{59 36r1N}", "36r1N"),
        ("{-10 :first -0xAN :second}", "-0xAN"),
        (
            "{123456789012345678901234567890N :first 123456789012345678901234567890 :second}",
            "123456789012345678901234567890",
        ),
    ] {
        for read in [reader::read, reader::read_edn] {
            let error = read(source).unwrap_err();
            assert!(error.message.starts_with("Duplicate"));
            assert_eq!(
                &source[error.span.start.offset..error.span.end.offset],
                duplicate
            );
        }
    }
    for read in [reader::read, reader::read_edn] {
        assert!(read("#{1 1.0 1M -1}").is_ok());
        assert!(read("[0xFF 255N 16rFF]").is_ok());
    }
}

#[test]
fn literal_numeric_duplicates_follow_jvm_numeric_categories() {
    for (source, repeated) in [
        ("#{0.0 -0.0}", "-0.0"),
        ("{1/2 :a 2/4 :b}", "2/4"),
        ("{1 :a 2/2 :b}", "2/2"),
        ("#{1.00M 100e-2M}", "100e-2M"),
        ("#{-0.000M 0M}", "0M"),
        ("#{##Inf 1e400}", "1e400"),
        (
            "#{340282366920938463463374607431768211456N 0x100000000000000000000000000000000}",
            "0x100000000000000000000000000000000",
        ),
        (
            "{1/2 :a 340282366920938463463374607431768211456/680564733841876926926749214863536422912 :b}",
            "340282366920938463463374607431768211456/680564733841876926926749214863536422912",
        ),
        (
            "#{340282366920938463463374607431768211456/3 680564733841876926926749214863536422912/6}",
            "680564733841876926926749214863536422912/6",
        ),
    ] {
        let parsed = reader::parse(source);
        assert_lossless(source, &parsed);
        for read in [reader::read, reader::read_edn] {
            let error = read(source).unwrap_err();
            assert!(
                error.message.starts_with("Duplicate"),
                "{source}: {error:?}"
            );
            assert_eq!(
                &source[error.span.start.offset..error.span.end.offset],
                repeated
            );
        }
    }
    for source in ["#{0 0.0 0M}", "#{1/2 0.5 0.5M}", "#{1M 1.0}"] {
        assert!(reader::read(source).is_ok(), "{source}");
        assert!(reader::read_edn(source).is_ok(), "{source}");
    }
}

#[test]
fn composite_literal_duplicates_use_reader_identity_for_nan_and_arguments() {
    for (source, repeated) in [
        ("#{[##NaN] (##NaN)}", "(##NaN)"),
        ("#{{:x ##NaN} {:x ##NaN}}", "{:x ##NaN}"),
        ("#{{[##NaN] :x} {[##NaN] :x}}", "{[##NaN] :x}"),
        ("#{#{##NaN} #{##NaN}}", "#{##NaN}"),
    ] {
        for read in [reader::read, reader::read_edn] {
            let error = read(source).unwrap_err();
            assert!(error.message.starts_with("Duplicate"), "{error:?}");
            assert_eq!(
                &source[error.span.start.offset..error.span.end.offset],
                repeated
            );
        }
    }
    let nan_key = "#{{##NaN :x} {##NaN :x}}";
    assert!(reader::read(nan_key).is_ok());
    assert!(reader::read_edn(nan_key).is_ok());

    let duplicate = "#(vector #{[%] [%1]})";
    let error = reader::read(duplicate).unwrap_err();
    assert_eq!(
        &duplicate[error.span.start.offset..error.span.end.offset],
        "[%1]"
    );
    assert!(reader::read("#(vector #{[%1] [%2]})").is_ok());
    assert!(reader::read("#{#(identity %) #(identity %)}").is_ok());
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

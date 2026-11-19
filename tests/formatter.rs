use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use parenox::formatter::{self, FormatOptions, Language};
use parenox::reader;
use parenox::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

#[derive(Debug, PartialEq, Eq)]
enum Shape<'a> {
    Enter(SyntaxKind),
    Token(SyntaxKind, &'a str),
    Exit,
}

fn shape(source: &str, language: Language) -> Vec<Shape<'_>> {
    fn collect<'a>(source: &'a str, node: SyntaxNode, target: &mut Vec<Shape<'a>>) {
        target.push(Shape::Enter(node.kind()));
        for element in node.children_with_tokens() {
            match element {
                SyntaxElement::Node(child) => collect(source, child, target),
                SyntaxElement::Token(token) if token.kind() != SyntaxKind::Whitespace => {
                    let range = token.text_range();
                    let text = &source[usize::from(range.start())..usize::from(range.end())];
                    target.push(Shape::Token(token.kind(), text));
                }
                _ => {}
            }
        }
        target.push(Shape::Exit);
    }
    let parsed = match language {
        Language::Clojure => reader::parse(source),
        Language::Edn => reader::parse_edn(source),
    };
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let mut result = Vec::new();
    collect(source, parsed.syntax(), &mut result);
    result
}

#[test]
fn formats_calls_bindings_and_data_without_reflowing_lines() {
    let source = " (defn  greet [name ]\n    (let [hello  (str \"Hello \"\n name)\n  options {:a[1 2],  :b\n3}]\n  (assoc options\n        :greeting hello)))  ";
    let expected = "(defn greet [name]\n  (let [hello (str \"Hello \"\n                   name)\n        options {:a [1 2], :b\n                 3}]\n    (assoc options\n           :greeting hello)))\n";
    let options = FormatOptions::default();
    let formatted = formatter::format(source, Language::Clojure, &options).unwrap();
    assert_eq!(formatted, expected);
    assert_eq!(
        shape(source, Language::Clojure),
        shape(&formatted, Language::Clojure)
    );
    assert_eq!(
        formatter::format(&formatted, Language::Clojure, &options).unwrap(),
        formatted
    );
}

#[test]
fn preserves_comments_discards_reader_prefixes_and_both_conditional_branches() {
    let source = "#!/usr/bin/env clojure\r\n;; keep this comment\r\n#_ (let [unused   42]\r\n ignored)\r\n#?(:clj  (str \"two\r\n  spaces\")\r\n :cljs #js {:demo  3})\r\n#:demo{:a  1, :b [^String fox \"🦊\"]}";
    let options = FormatOptions::default();
    let formatted = formatter::format(source, Language::Clojure, &options).unwrap();
    assert!(formatted.contains("#_ (let [unused   42]\r\n ignored)"));
    assert!(formatted.contains("(str \"two\r\n  spaces\")"));
    assert!(formatted.contains("\r\n   :cljs #js {:demo 3})"));
    assert!(formatted.ends_with("#:demo{:a 1, :b [^String fox \"🦊\"]}\r\n"));
    assert_eq!(
        shape(source, Language::Clojure),
        shape(&formatted, Language::Clojure)
    );
    assert_eq!(
        formatter::format(&formatted, Language::Clojure, &options).unwrap(),
        formatted
    );
}

#[test]
fn formats_edn_with_its_own_reader_policy_and_declines_invalid_source() {
    let options = FormatOptions::default();
    let source = "{  :point #demo/point [1  2]\n   :tag #^String item }";
    let formatted = formatter::format(source, Language::Edn, &options).unwrap();
    assert_eq!(
        formatted,
        "{:point #demo/point [1 2]\n :tag #^String item}\n"
    );
    assert_eq!(
        shape(source, Language::Edn),
        shape(&formatted, Language::Edn)
    );
    assert!(formatter::format("{:x @thing}", Language::Edn, &options).is_err());
    assert!(formatter::format("(defn f [x] x", Language::Clojure, &options).is_err());
    assert_eq!(
        formatter::format("(fn x\n  y)", Language::Edn, &options).unwrap(),
        "(fn x\n y)\n"
    );
}

#[test]
fn preserves_distinct_line_breaks_in_a_mixed_endings_file() {
    let source = "{:a 1\r\t  \n  :b 2}";
    assert_eq!(
        formatter::format(source, Language::Edn, &FormatOptions::default()).unwrap(),
        "{:a 1\r \n :b 2}\r"
    );
}

#[test]
fn formats_selected_lines_without_merging_breaks_or_rewriting_literals() {
    let source = "  (list \"first\r\n   second\"\r\t  \n  :tail)\r\n(def   untouched 1)";
    let next_line = source.find("\r\t").unwrap() + 1;
    let unselected = source.find("(def").unwrap();
    let options = FormatOptions::default();
    let formatted =
        formatter::format_lines(source, Language::Clojure, &options, next_line..unselected)
            .unwrap();
    assert_eq!(
        formatted,
        "  (list \"first\r\n   second\"\r \n        :tail)\r\n(def   untouched 1)"
    );
    assert_eq!(
        shape(source, Language::Clojure),
        shape(&formatted, Language::Clojure)
    );
}

#[test]
#[ignore = "requires a local Clojure project checkout"]
fn pilot_formatting_is_lossless_and_idempotent() {
    fn visit(path: &Path, files: &mut Vec<PathBuf>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        if metadata.is_symlink() {
            return;
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                if !matches!(
                    entry.file_name().to_str(),
                    Some(".git" | ".cpcache" | "target")
                ) {
                    visit(&entry.path(), files);
                }
            }
        } else if Language::for_path(path).is_some() {
            files.push(path.into());
        }
    }
    let root =
        PathBuf::from(std::env::var_os("PARENOX_FORMAT_PILOT").expect("set PARENOX_FORMAT_PILOT"));
    let mut files = Vec::new();
    visit(&root, &mut files);
    files.sort();
    let mut checked = 0;
    let mut changed = 0;
    let mut rejected = 0;
    let mut elapsed = std::time::Duration::ZERO;
    let options = FormatOptions::default();
    for file in files {
        let source = fs::read_to_string(&file).unwrap();
        let language = Language::for_path(&file).unwrap();
        let started = Instant::now();
        let formatted = formatter::format(&source, language, &options);
        elapsed += started.elapsed();
        let formatted = match formatted {
            Ok(text) => text,
            Err(error) => {
                rejected += 1;
                eprintln!("Reader declined {}: {}", file.display(), error.message);
                continue;
            }
        };
        checked += 1;
        changed += usize::from(formatted != source);
        assert!(
            shape(&formatted, language) == shape(&source, language),
            "the syntax tree changed in {}",
            file.display()
        );
        let repeated = formatter::format(&formatted, language, &options).unwrap();
        assert_eq!(repeated, formatted, "{}", file.display());
    }
    eprintln!(
        "{checked} files checked, {changed} changed, {rejected} declined; {elapsed:?} in formatter"
    );
    assert!(checked > 0, "the checkout has no parseable inputs");
    assert_eq!(rejected, 0, "the reader declined some files");
}

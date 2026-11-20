use std::fs;
use std::process::Command;

use parenox_lsp::text::{Lines, Range};
use serde_json::json;

use super::{Fixture, transcript, uri};

#[test]
fn cli_and_range_formatting_agree_on_a_configured_clojure_test_macro() {
    let fixture = Fixture::new("portable macro formatting");
    let path = fixture.0.join("src/macro_example.cljc");
    let source = include_str!("../fixtures/formatting_macro.cljc");
    fs::write(&path, source).unwrap();
    fs::write(
        fixture.0.join("parenox.edn"),
        "{:format {:indent-width 4 :indent-rules {clojure.test/testing :block}}}",
    )
    .unwrap();
    let cli = |options: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_parenox"))
            .current_dir(&fixture.0)
            .arg("fmt")
            .args(options)
            .arg(&path)
            .output()
            .unwrap()
    };
    let output = cli(&[]);
    assert!(output.status.success(), "{output:?}");
    let formatted = String::from_utf8(output.stdout).unwrap();
    assert!(formatted.contains("\n    #?(:clj (clojure.test/is (= [1 2] [1 2]))"));
    assert!(formatted.contains(":cljs (clojure.test/is (= [3 4] [3 4]))"));
    for text in [
        ";; Keep the dropped form next to the checks.",
        "#_(clojure.test/is    false)",
    ] {
        assert!(formatted.contains(text), "{formatted}");
    }
    fs::write(&path, &formatted).unwrap();
    let checked = cli(&["--check"]);
    assert!(checked.status.success(), "{checked:?}");

    let file = uri(&path);
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":file,"version":1,"languageId":"clojure","text":source}
        }}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/rangeFormatting","params":{
            "textDocument":{"uri":file},
            "range":{"start":{"line":2,"character":0},"end":{"line":7,"character":0}},
            "options":{"tabSize":2,"insertSpaces":true}
        }}),
        json!({"jsonrpc":"2.0","id":3,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let edits = messages.iter().find(|message| message["id"] == 2).unwrap()["result"]
        .as_array()
        .unwrap();
    assert!(!edits.is_empty());
    let lines = Lines::new(source);
    let mut result = source.to_owned();
    for edit in edits.iter().rev() {
        let range: Range = serde_json::from_value(edit["range"].clone()).unwrap();
        let start = lines.offset(range.start).unwrap();
        let end = lines.offset(range.end).unwrap();
        result.replace_range(start..end, edit["newText"].as_str().unwrap());
    }
    assert_eq!(
        result,
        formatted.replace("(def unchanged 1)", "(def   unchanged  1)")
    );
    assert_eq!(fs::read_to_string(path).unwrap(), formatted);
}

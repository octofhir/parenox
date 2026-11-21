use super::*;

#[test]
fn method_body_checks_a_string_call_and_hovers_its_reached_result() {
    let fixture = Fixture::new("multimethod types");
    let file = fixture.0.join("src/description.clj");
    let document = uri(&file);
    let source = r#"(ns description (:require [clojure.string :as str]))
(defprotocol Surface (describe [schema]))
(defmulti render (fn [tag children] tag))
(defmethod render :or [_ children]
  (let [description (str/join ", or " children)]
    description))
(defmethod render :throws [_ _]
  (throw (ex-info "abort" {}))
  (str/join ", " 42))
(defmethod render :opaque [_ configure]
  (configure)
  (str/join ", " 42))
"#;
    let invalid = source.replace("children)]", "42)]");
    fs::write(&file, &invalid).unwrap();
    let cli = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .current_dir(&fixture.0)
        .args(["check", "--allow-incomplete", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(cli.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&cli.stdout).unwrap();
    let file_report = report["files"]
        .as_array()
        .and_then(|files| files.first())
        .unwrap_or(&report);
    let errors: Vec<_> = file_report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == "PNX2322")
        .collect();
    let [error] = errors.as_slice() else {
        panic!("{report}");
    };
    let span = &error["span"];
    let start = span["start"]["offset"].as_u64().unwrap() as usize;
    let end = span["end"]["offset"].as_u64().unwrap() as usize;
    assert_eq!(&invalid[start..end], "42");
    assert_eq!(span["start"]["line"], 5);
    assert_eq!(error["contract"]["name"], "clojure.string/join");
    assert_eq!(file_report["complete"], false);
    let line = invalid.lines().nth(4).unwrap();
    let column = line[..line.find("42").unwrap()].encode_utf16().count();
    fs::write(&file, source).unwrap();

    let hover = |id| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/hover","params":{
            "textDocument":{"uri":document},"position":{"line":5,"character":4}
        }})
    };
    let change = |version, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let responses = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":document,"languageId":"clojure","version":1,"text":source}
        }}),
        hover(2),
        change(2, &invalid),
        hover(3),
        change(3, source),
        hover(4),
        json!({"jsonrpc":"2.0","id":5,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| &responses.iter().find(|reply| reply["id"] == id).unwrap()["result"];
    assert_eq!(result(2)["contents"]["value"], "description\nString");
    assert!(result(3).is_null(), "{}", result(3));
    assert_eq!(result(2), result(4));
    for (version, expected) in [(1, 0), (2, 1), (3, 0)] {
        let publication = responses
            .iter()
            .find(|message| {
                message["method"] == "textDocument/publishDiagnostics"
                    && message["params"]["version"] == version
            })
            .unwrap();
        let findings: Vec<_> = publication["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|finding| finding["code"] == "PNX2322")
            .collect();
        assert_eq!(findings.len(), expected, "{publication}");
        if let [finding] = findings.as_slice() {
            assert_eq!(finding["message"], error["message"]);
            assert_eq!(
                finding["range"],
                json!({
                    "start":{"line":4,"character":column},
                    "end":{"line":4,"character":column + 2}
                })
            );
        }
    }
}

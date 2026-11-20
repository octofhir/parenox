use super::*;

#[test]
fn hover_uses_the_type_at_a_local_read_after_an_unsaved_edit() {
    let fixture = Fixture::new("local hover");
    let path = fixture.0.join("src/uri_path.cljc");
    let document = uri(&path);
    // The nil-path branch follows lambdaisland.uri/join*.
    let source = r#"(ns uri-path (:require [clojure.spec.alpha :as s] [clojure.string :as str]))
(defn normalize {:t/sig '[{:path (s/nilable string?)} :-> string?]} [uri]
  (let [path (:path uri)]
    (if (nil? (:path uri))
      (str path)
      (do "🦊" (subs path 0)))))
"#;
    fs::write(&path, source).unwrap();
    let broken = source.replace("(nil? (:path uri))", "(some? (:path uri))");
    let column = |line: usize| {
        let row = source.lines().nth(line).unwrap();
        row[..row.rfind("path").unwrap()].encode_utf16().count()
    };
    let hover = |id, line| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/hover","params":{
            "textDocument":{"uri":document},
            "position":{"line":line,"character":column(line)+1}
        }})
    };
    let change = |version, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let responses = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"languageId":"clojure","version":1,"text":source
        }}}),
        hover(2, 4),
        hover(3, 5),
        change(2, &broken),
        hover(4, 4),
        hover(5, 5),
        change(3, source),
        hover(6, 5),
        json!({"jsonrpc":"2.0","id":7,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let range = |line| {
        json!({
            "start":{"line":line,"character":column(line)},
            "end":{"line":line,"character":column(line)+4}
        })
    };
    for (id, line, ty) in [
        (2, 4, "Nil"),
        (3, 5, "String"),
        (4, 4, "String"),
        (5, 5, "Nil"),
        (6, 5, "String"),
    ] {
        let result = &responses
            .iter()
            .find(|message| message["id"] == id)
            .unwrap()["result"];
        assert_eq!(result["contents"]["value"], format!("path\n{ty}"));
        assert_eq!(result["range"], range(line));
    }
    let findings = |version| {
        let message = responses
            .iter()
            .find(|message| {
                message["method"] == "textDocument/publishDiagnostics"
                    && message["params"]["version"] == version
            })
            .unwrap();
        message["params"]["diagnostics"].as_array().unwrap()
    };
    assert!(findings(1).is_empty());
    let [diagnostic] = findings(2).as_slice() else {
        panic!("unexpected diagnostics: {:?}", findings(2));
    };
    assert_eq!(diagnostic["range"], range(5));
    assert_eq!(diagnostic["code"], "PNX2322");
    assert!(
        diagnostic["message"]
            .as_str()
            .unwrap()
            .contains("expected String, got Nil")
    );
    assert!(findings(3).is_empty());
    assert_eq!(fs::read_to_string(&path).unwrap(), source);

    fs::write(&path, &broken).unwrap();
    let cli = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .current_dir(&fixture.0)
        .args(["check", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(cli.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&cli.stdout).unwrap();
    assert_eq!(report["complete"], true);
    let findings = report["diagnostics"].as_array().unwrap();
    assert_eq!(findings.len(), 1);
    let cli = &findings[0];
    assert_eq!(cli["code"], diagnostic["code"]);
    assert_eq!(cli["message"], diagnostic["message"]);
    assert_eq!(cli["expected"], "String");
    assert_eq!(cli["actual"], "Nil");
    let start = cli["span"]["start"]["offset"].as_u64().unwrap() as usize;
    assert_eq!(start, broken.rfind("path").unwrap());
}

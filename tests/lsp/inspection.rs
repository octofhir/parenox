use std::fs;

use serde_json::{Value, json};

use super::{Fixture, transcript, uri};

#[test]
fn inspects_live_source_and_workspace_from_raw_methods_and_commands() {
    let fixture = Fixture::new("inspection");
    let path = fixture.0.join("src/inspect.clj");
    fs::write(&path, "(ns inspect)\n").unwrap();
    let root = uri(&fixture.0);
    let document = uri(&path);
    let source = "(ns inspect)\n\n(defn greet [name]\n  (str \"🌿\" name))\n(greet \"friend\")\n";
    let cursor = |line, character| json!({"textDocument":{"uri":document},"position":{"line":line,"character":character}});
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":root}}),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":document,"languageId":"clojure","version":1,"text":source}
        }}),
        json!({"jsonrpc":"2.0","id":2,"method":"clojure/serverInfo/raw","params":{}}),
        json!({"jsonrpc":"2.0","id":3,"method":"clojure/cursorInfo/raw","params":cursor(3,13)}),
        json!({"jsonrpc":"2.0","id":4,"method":"clojure/cursorInfo/raw","params":cursor(1,0)}),
        json!({"jsonrpc":"2.0","id":5,"method":"clojure/cursorInfo/raw","params":cursor(2,7)}),
        json!({"jsonrpc":"2.0","id":6,"method":"clojure/cursorInfo/raw","params":cursor(2,14)}),
        json!({"jsonrpc":"2.0","id":7,"method":"clojure/cursorInfo/raw","params":cursor(4,2)}),
        json!({"jsonrpc":"2.0","id":8,"method":"clojure/cursorInfo/raw","params":cursor(3,4)}),
        json!({"jsonrpc":"2.0","method":"clojure/serverInfo/log","params":{}}),
        json!({"jsonrpc":"2.0","method":"clojure/cursorInfo/log","params":cursor(3,13)}),
        json!({"jsonrpc":"2.0","id":9,"method":"workspace/executeCommand","params":{
            "command":"server-info","arguments":[document,0,0]
        }}),
        json!({"jsonrpc":"2.0","id":10,"method":"workspace/executeCommand","params":{
            "command":"cursor-info","arguments":[document,0,0,4,14]
        }}),
        json!({"jsonrpc":"2.0","id":11,"method":"workspace/executeCommand","params":{
            "command":"cursor-info","arguments":[document,0,0,0,14]
        }}),
        json!({"jsonrpc":"2.0","id":12,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| messages.iter().find(|message| message["id"] == id).unwrap();
    let result = |id| &response(id)["result"];
    let capabilities = &result(1)["capabilities"];
    assert_eq!(capabilities["experimental"]["cursorInfo"], true);
    assert_eq!(capabilities["experimental"]["serverInfo"], true);
    assert_eq!(
        capabilities["executeCommandProvider"]["commands"],
        json!([
            "server-info",
            "cursor-info",
            "cycle-privacy",
            "cycle-keyword-auto-resolve",
            "thread-first",
            "thread-first-all",
            "thread-last",
            "thread-last-all",
            "unwind-thread",
            "unwind-all",
            "cycle-coll",
            "change-coll",
            "if->cond-refactor",
            "cond->if-refactor",
            "get-in-all",
            "get-in-less",
            "get-in-none"
        ])
    );
    let server = result(2);
    assert_eq!(server["project-root-uri"], root);
    assert_eq!(server["analysis-summary"]["internal"]["indexed-files"], 1);
    assert_eq!(server["analysis-summary"]["open-documents"], 1);
    assert!(
        server["classpath"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path.as_str().is_some_and(|path| path.ends_with("/src")))
    );
    assert_eq!(result(3)["node"]["string-value"], "name");
    let local = &result(3)["elements"][0];
    assert_eq!(local["element"]["bucket"], "local-usages");
    assert_eq!(local["element"]["name-col"], 13);
    assert_eq!(local["definition"]["bucket"], "locals");
    assert_eq!(local["definition"]["name-row"], 3);
    assert_eq!(local["definition"]["name-col"], 14);
    assert_eq!(local["semantic-tokens"][0]["token-type"], "variable");
    assert_eq!(result(4)["elements"], json!([]));
    let declaration = &result(5)["elements"][0];
    assert_eq!(declaration["element"]["bucket"], "var-definitions");
    assert_eq!(declaration["element"], declaration["definition"]);
    assert_eq!(
        declaration["semantic-tokens"][0]["token-modifier"],
        json!(["definition"])
    );
    assert_eq!(result(6)["elements"][0]["element"]["bucket"], "locals");
    let reference = &result(7)["elements"][0];
    assert_eq!(reference["element"]["bucket"], "var-usages");
    assert_eq!(reference["definition"], declaration["definition"]);
    assert_eq!(result(8)["elements"][0]["element"]["to"], "clojure.core");
    assert_eq!(result(9), &Value::Null);
    assert_eq!(result(10), &Value::Null);
    assert_eq!(response(11)["error"]["code"], -32602);
    let logs: Vec<_> = messages
        .iter()
        .filter(|message| message["method"] == "window/showMessage")
        .collect();
    assert_eq!(logs.len(), 4);
    for message in &logs {
        assert_eq!(message["params"]["type"], 3);
    }
    let text = |i: usize| logs[i]["params"]["message"].as_str().unwrap();
    assert!(text(0).contains("project-root-uri"));
    assert!(text(1).contains("local-usages"));
    assert_eq!(text(0), text(2));
    assert_eq!(text(1), text(3));
}

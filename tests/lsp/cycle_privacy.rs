use super::*;
use parenox_lsp::text::{Lines, Range};

const SOURCE: &str = include_str!("../fixtures/cycle_privacy.clj");

fn cursor(source: &str, line: usize, word: &str) -> usize {
    let row = source.lines().nth(line).unwrap();
    row[..row.rfind(word).unwrap()].encode_utf16().count()
}

fn command(id: usize, file: &str, line: usize, character: usize) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"workspace/executeCommand","params":{
        "command":"cycle-privacy","arguments":[file,line,character]
    }})
}

fn apply(source: &str, edits: &Value) -> String {
    let lines = Lines::new(source);
    let mut result = source.to_owned();
    for edit in edits.as_array().unwrap().iter().rev() {
        let range: Range = serde_json::from_value(edit["range"].clone()).unwrap();
        let span = lines.byte_range(range).unwrap();
        result.replace_range(span, edit["newText"].as_str().unwrap());
    }
    result
}

#[test]
fn cycle_privacy_returns_lossless_rewrites_and_inline_code_actions() {
    let fixture = Fixture::new("cycle privacy");
    let path = fixture.0.join("src/demo.clj");
    let file = uri(&path);
    fs::write(&path, SOURCE).unwrap();
    let requests = [
        (1, "expose", "(defn expose", "(defn- expose"),
        (2, "hidden", "(defn- hidden", "(defn hidden"),
        (3, "secret", "(def ^:private secret", "(def secret"),
        (
            4,
            "label",
            "(def ^String label",
            "(def ^:private ^String label",
        ),
        (
            5,
            "double-private",
            "(defn- ^:private double-private",
            "(defn double-private",
        ),
        (6, "flagged", "{:private false", "{:private true"),
        (7, "instance", "{:private true", "{:private false"),
    ];
    let pos = json!({"line":1,"character":cursor(SOURCE,1,"expose")});
    let mut messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":file,"version":2,"languageId":"clojure","text":SOURCE}
        }}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/codeAction","params":{
            "textDocument":{"uri":file},"range":{"start":pos,"end":pos},
            "context":{"diagnostics":[],"only":["refactor"]}
        }}),
    ];
    messages.extend(
        requests.iter().enumerate().map(|(i, (line, name, _, _))| {
            command(i + 3, &file, *line, cursor(SOURCE, *line, name))
        }),
    );
    messages.extend([
        command(11, &file, 8, cursor(SOURCE, 8, "discarded")),
        json!({"jsonrpc":"2.0","id":12,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let messages = transcript(&messages);
    let response = |id| messages.iter().find(|message| message["id"] == id).unwrap();
    let action = response(2)["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["title"] == "Cycle privacy")
        .unwrap();
    assert_eq!(action["kind"], "refactor.rewrite");
    assert_eq!(action["edit"], response(3)["result"]);
    for (i, (_, _, old, new)) in requests.iter().enumerate() {
        let edits = &response(i + 3)["result"]["changes"][&file];
        assert_eq!(apply(SOURCE, edits), SOURCE.replacen(old, new, 1));
    }
    assert_eq!(response(11)["error"]["code"], -32600);
    assert!(
        messages
            .iter()
            .all(|message| message["method"] != "workspace/applyEdit")
    );
    assert_eq!(fs::read_to_string(path).unwrap(), SOURCE);
}

#[test]
fn cycle_privacy_waits_for_the_matching_client_edit_response() {
    let fixture = Fixture::new("cycle privacy handshake");
    let path = fixture.0.join("src/demo.clj");
    let file = uri(&path);
    let overlay = SOURCE.replace("defn expose", "defn- unsaved");
    fs::write(&path, SOURCE).unwrap();
    let mut child = lsp_server();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut next = || loop {
        let message = read_message(&mut output).expect("LSP message");
        if message.get("id").is_some() {
            break message;
        }
    };
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&fixture.0),"capabilities":{"workspace":{"applyEdit":true,"workspaceEdit":{"documentChanges":true}}}
        }}),
    );
    assert_eq!(next()["id"], 1);
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":file,"version":1,"languageId":"clojure","text":SOURCE}
        }}),
    );
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":7},"contentChanges":[{"text":overlay}]
        }}),
    );
    send_message(
        &mut input,
        &command(2, &file, 1, cursor(&overlay, 1, "unsaved")),
    );
    let first = next();
    assert_eq!(first["method"], "workspace/applyEdit", "{first}");
    let edit = &first["params"]["edit"];
    let change = &edit["documentChanges"][0];
    let start = cursor(&overlay, 1, "defn-");
    assert_eq!(
        change,
        &json!({
            "textDocument":{"uri":file,"version":7},"edits":[{
                "range":{"start":{"line":1,"character":start},"end":{"line":1,"character":start+5}},
                "newText":"defn"
            }]
        })
    );
    send_message(
        &mut input,
        &command(3, &file, 3, cursor(&overlay, 3, "secret")),
    );
    let second = next();
    assert_eq!(second["method"], "workspace/applyEdit");
    assert_ne!(first["id"], second["id"]);
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","id":second["id"],"result":{
            "applied":false,"failureReason":"version conflict"
        }}),
    );
    let rejected = next();
    assert_eq!(rejected["id"], 3);
    assert_eq!(
        rejected["error"],
        json!({"code":-32803,"message":"version conflict"})
    );
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","id":first["id"],"result":{"applied":true}}),
    );
    let applied = next();
    assert_eq!(applied["id"], 2);
    assert_eq!(&applied["result"], edit);
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","id":4,"method":"shutdown"}),
    );
    assert_eq!(next()["id"], 4);
    send_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
    assert_eq!(fs::read_to_string(path).unwrap(), SOURCE);
}

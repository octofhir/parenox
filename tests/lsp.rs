use std::fs;
use std::io::{BufRead, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("parenox lsp {}", std::process::id()));
        fs::create_dir(&path).unwrap();
        fs::create_dir(path.join("src")).unwrap();
        fs::create_dir(path.join("test")).unwrap();
        fs::write(
            path.join("deps.edn"),
            r#"{:paths ["src"] :aliases {:test {:extra-paths ["test"]}}}"#,
        )
        .unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn uri(path: &Path) -> String {
    let path = path
        .to_str()
        .unwrap()
        .replace('\\', "/")
        .replace(' ', "%20");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

fn transcript(messages: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args(["lsp", "--alias", "test"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut input = child.stdin.take().unwrap();
        for message in messages {
            let body = serde_json::to_vec(message).unwrap();
            write!(input, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
            input.write_all(&body).unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut reader = Cursor::new(output.stdout);
    let mut messages = Vec::new();
    let mut header = String::new();
    while reader.read_line(&mut header).unwrap() != 0 {
        let length = header
            .strip_prefix("Content-Length: ")
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        header.clear();
        reader.read_line(&mut header).unwrap();
        assert_eq!(header, "\r\n");
        header.clear();
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        messages.push(serde_json::from_slice(&body).unwrap());
    }
    messages
}

#[test]
fn editor_uses_aliases_overlays_and_utf16_ranges() {
    let fixture = Fixture::new();
    let provider = fixture.0.join("src/shop store.clj");
    let caller = fixture.0.join("test/caller.cljc");
    let external = fixture.0.join("test/unsaved.clj");
    fs::write(
        &provider,
        "(ns shop.store)\r\n(defn ^String title {:t/sig '[string? :-> string?]} [id] id)\r\n",
    )
    .unwrap();
    let app = "(ns shop.test (:require [shop.store :as store]))\r\n(defn label {:t/sig '[:-> string?]} [] (do \"🦀\" (store/title 42)))\r\n";
    let fixed = app.replace("42", "\"id\"");
    fs::write(&caller, &fixed).unwrap();
    let line = app.lines().nth(1).unwrap();
    let column = |word| line[..line.find(word).unwrap()].encode_utf16().count();
    let query = |id| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/definition","params":{
            "textDocument":{"uri":uri(&caller)}, "position":{"line":1,"character":column("store/title")}
        }})
    };
    let change = |version, text| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri(&caller),"version":version}, "contentChanges":[{"text":text}]
        }})
    };
    let messages = transcript(&[
        query(0),
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0),"capabilities":{}}}),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":uri(&caller),"languageId":"clojure","version":1,"text":app
        }}}),
        query(2),
        change(3, &fixed),
        change(2, &"(ns stale)".into()),
        query(3),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":uri(&external),"languageId":"clojure","version":1,"text":"(ns broken"
        }}}),
        query(4),
        json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":uri(&external)}}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":uri(&caller)}}}),
        json!({"jsonrpc":"2.0","id":5,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    assert_eq!(messages.len(), 11, "{messages:?}");
    assert_eq!(messages[0]["error"]["code"], -32002);
    let capabilities = &messages[1]["result"]["capabilities"];
    assert_eq!(capabilities["definitionProvider"], true);
    assert_eq!(capabilities["positionEncoding"], "utf-16");
    assert_eq!(capabilities["textDocumentSync"]["change"], 1);
    let reported = &messages[2]["params"];
    assert_eq!(reported["version"], 1);
    assert_eq!(reported["diagnostics"][0]["code"], "PNX2322");
    assert_eq!(
        reported["diagnostics"][0]["range"]["start"],
        json!({"line":1,"character":column("42")})
    );
    let target = json!({"uri":uri(&provider),"range":{"start":{"line":1,"character":14},"end":{"line":1,"character":19}}});
    for index in [3, 5, 7] {
        assert_eq!(messages[index]["result"], target);
    }
    assert_eq!(messages[4]["params"]["version"], 3);
    assert_eq!(messages[4]["params"]["diagnostics"], json!([]));
    assert_eq!(messages[6]["params"]["diagnostics"][0]["code"], "PNX9000");
    for index in [8, 9] {
        assert_eq!(messages[index]["params"]["diagnostics"], json!([]));
    }
    assert_eq!(messages[10]["id"], 5);
    assert!(messages[10]["result"].is_null());
}

use std::fs;
use std::io::{BufRead, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("parenox lsp {name} {}", std::process::id()));
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
    let mut input = child.stdin.take().unwrap();
    let output = std::thread::scope(|scope| {
        scope.spawn(move || {
            for message in messages {
                let body = serde_json::to_vec(message).unwrap();
                write!(input, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
                input.write_all(&body).unwrap();
            }
        });
        child.wait_with_output().unwrap()
    });
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
    let fixture = Fixture::new("source");
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
    let unfinished = format!("{fixed}(defn unfinished [");
    fs::write(&caller, &fixed).unwrap();
    let line = app.lines().nth(1).unwrap();
    let column = |word| line[..line.find(word).unwrap()].encode_utf16().count();
    let query = |id| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/definition","params":{
            "textDocument":{"uri":uri(&caller)}, "position":{"line":1,"character":column("store/title")}
        }})
    };
    let mut hover = query(6);
    hover["method"] = "textDocument/hover".into();
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
        hover,
        change(3, &fixed),
        change(2, &"(ns stale)".to_owned()),
        query(3),
        change(4, &unfinished),
        query(7),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":uri(&external),"languageId":"clojure","version":1,"text":"(ns broken"
        }}}),
        query(4),
        json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":uri(&external)}}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":uri(&caller)}}}),
        json!({"jsonrpc":"2.0","id":5,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    assert_eq!(messages.len(), 14, "{messages:?}");
    assert_eq!(messages[0]["error"]["code"], -32002);
    let capabilities = &messages[1]["result"]["capabilities"];
    assert_eq!(capabilities["definitionProvider"], true);
    assert_eq!(capabilities["hoverProvider"], true);
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
    for index in [3, 6, 8, 10] {
        assert_eq!(messages[index]["result"], target);
    }
    let hover = messages[4]["result"]["contents"]["value"].as_str().unwrap();
    assert!(
        hover.contains("shop.store/title\n[String -> String]"),
        "{hover}"
    );
    assert!(hover.contains(":t/sig"), "{hover}");
    assert_eq!(messages[5]["params"]["version"], 3);
    assert_eq!(messages[5]["params"]["diagnostics"], json!([]));
    for index in [7, 9] {
        assert_eq!(
            messages[index]["params"]["diagnostics"][0]["code"],
            "PNX9000"
        );
    }
    for index in [11, 12] {
        assert_eq!(messages[index]["params"]["diagnostics"], json!([]));
    }
    assert_eq!(messages[13]["id"], 5);
    assert!(messages[13]["result"].is_null());
}

#[test]
fn edn_buffers_get_reader_diagnostics_and_clear_on_edit() {
    let fixture = Fixture::new("edn");
    let document = uri(&fixture.0.join("options.edn"));
    let responses = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0),"capabilities":{}}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"languageId":"edn","version":1,"text":"{:match #\"x\"}"
        }}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":2},
            "contentChanges":[{"text":"#:config{:match #vendor/tag [\"x\" ##Inf]}"}]
        }}),
        json!({"jsonrpc":"2.0","id":2,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    assert_eq!(responses.len(), 4, "{responses:?}");
    assert_eq!(responses[1]["params"]["diagnostics"][0]["code"], "PNX9000");
    assert_eq!(
        responses[1]["params"]["diagnostics"][0]["range"]["start"],
        json!({"line":0,"character":8})
    );
    assert_eq!(responses[2]["params"]["version"], 2);
    assert_eq!(responses[2]["params"]["diagnostics"], json!([]));
}

#[test]
fn untyped_call_typo_matches_cli_and_clears_on_edit() {
    let fixture = Fixture::new("untyped");
    let caller = fixture.0.join("test/uri.cljc");
    let provider = fixture.0.join("src/normalize.cljc");
    fs::write(
        &caller,
        "(ns uri (:require [normalize :as normalize]))\n(defn decode [x]\n  (let [s x]\n    [(if s (normalize/percent-decode s) \"\")]))\n",
    ).unwrap();
    fs::write(&provider, "(ns normalize)\n(defn percent-decode [s] s)\n").unwrap();
    change_call(&fixture.0, &caller, &provider, 4, 2);
}

#[test]
#[ignore = "requires the pinned lambdaisland/uri checkout"]
fn uri_pilot_editor_workflow() {
    let root = PathBuf::from(std::env::var_os("PARENOX_URI_PILOT").expect("set PARENOX_URI_PILOT"));
    change_call(
        &root,
        &root.join("src/lambdaisland/uri.cljc"),
        &root.join("src/lambdaisland/uri/normalize.cljc"),
        134,
        82,
    );
}

fn change_call(root: &Path, caller: &Path, provider: &Path, call_line: usize, target_line: usize) {
    let document = uri(caller);
    let source = fs::read_to_string(caller).unwrap();
    let line = source.lines().nth(call_line - 1).unwrap();
    let correct = "normalize/percent-decode";
    let typo = "normalize/percent-decoe";
    let column = line.find(correct).unwrap();
    let broken = source.replacen(correct, typo, 1);
    assert!(broken.lines().nth(call_line - 1).unwrap().contains(typo));
    let change = |version, source| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":[{"text":source}]
        }})
    };
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(root),"capabilities":{}}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"languageId":"clojure","version":1,"text":source
        }}}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/definition","params":{
            "textDocument":{"uri":document},"position":{"line":call_line - 1,"character":column}
        }}),
        change(2, &broken),
        change(3, &source),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":uri(&root.join("trial.edn")),"languageId":"edn","version":1,"text":"{:enabled [true}"
        }}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri(&root.join("trial.edn")),"version":2},
            "contentChanges":[{"text":"{:enabled [true]}"}]
        }}),
        json!({"jsonrpc":"2.0","id":3,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    assert_eq!(messages.len(), 8, "{messages:?}");
    assert_eq!(messages[2]["result"]["uri"], uri(provider));
    assert_eq!(
        messages[2]["result"]["range"]["start"]["line"],
        target_line - 1
    );
    assert_eq!(messages[3]["params"]["version"], 2);
    assert_eq!(messages[4]["params"]["version"], 3);
    assert_eq!(
        messages[1]["params"]["diagnostics"],
        messages[4]["params"]["diagnostics"]
    );
    let diagnostics = messages[3]["params"]["diagnostics"].as_array().unwrap();
    let issues: Vec<_> = diagnostics
        .iter()
        .filter(|d| d["code"] == "PNX2304")
        .collect();
    assert_eq!(issues.len(), 1, "{diagnostics:?}");
    assert_eq!(
        issues[0]["range"]["start"],
        json!({"line":call_line - 1,"character":column})
    );
    assert_eq!(messages[5]["params"]["diagnostics"][0]["code"], "PNX9000");
    assert_eq!(messages[6]["params"]["diagnostics"], json!([]));
    assert_eq!(fs::read_to_string(caller).unwrap(), source);

    // Run a fresh CLI process against an independent copy of the broken snapshot.
    let fixture = Fixture::new("cli parity");
    copy_sources(root, &fixture.0, Path::new("src"));
    copy_sources(root, &fixture.0, Path::new("test"));
    let relative = caller.strip_prefix(root).unwrap();
    let cli_caller = fixture.0.join(relative);
    fs::write(&cli_caller, broken).unwrap();
    let checked = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .current_dir(&fixture.0)
        .args(["check", "--alias", "test", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(checked.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(report["complete"], false);
    let cli_file = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| Path::new(file["path"].as_str().unwrap()).ends_with(relative))
        .unwrap();
    let cli_diagnostics = cli_file["diagnostics"].as_array().unwrap();
    assert_eq!(cli_diagnostics.len(), diagnostics.len());
    for (lsp, cli) in diagnostics.iter().zip(cli_diagnostics) {
        assert_eq!(lsp["code"], cli["code"]);
        assert_eq!(lsp["message"], cli["message"]);
        assert_eq!(
            lsp["range"]["start"]["line"],
            cli["span"]["start"]["line"].as_u64().unwrap() - 1
        );
        assert_eq!(
            lsp["range"]["start"]["character"],
            cli["span"]["start"]["column"].as_u64().unwrap() - 1
        );
    }
}

fn copy_sources(from: &Path, to: &Path, relative: &Path) {
    let output = to.join(relative);
    fs::create_dir_all(&output).unwrap();
    for child in fs::read_dir(from.join(relative)).unwrap() {
        let child = child.unwrap();
        let kind = child.file_type().unwrap();
        if kind.is_dir() {
            copy_sources(from, to, &relative.join(child.file_name()));
        } else if kind.is_file() {
            fs::copy(child.path(), output.join(child.file_name())).unwrap();
        }
    }
}

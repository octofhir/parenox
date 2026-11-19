use std::fs;
use std::io::{BufRead, BufReader, Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

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

fn lsp_server() -> Child {
    lsp_server_with(Path::new(env!("CARGO_BIN_EXE_parenox")))
}

fn lsp_server_with(binary: &Path) -> Child {
    Command::new(binary)
        .args(["lsp", "--alias", "test"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn send_message(input: &mut impl Write, message: &Value) {
    let body = serde_json::to_vec(message).unwrap();
    write!(input, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
    input.write_all(&body).unwrap();
}

fn read_message(output: &mut impl BufRead) -> Option<Value> {
    let mut header = String::new();
    if output.read_line(&mut header).unwrap() == 0 {
        return None;
    }
    let length = header
        .strip_prefix("Content-Length: ")
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    header.clear();
    output.read_line(&mut header).unwrap();
    assert_eq!(header, "\r\n");
    let mut body = vec![0; length];
    output.read_exact(&mut body).unwrap();
    Some(serde_json::from_slice(&body).unwrap())
}

fn transcript(messages: &[Value]) -> Vec<Value> {
    let mut child = lsp_server();
    let mut input = child.stdin.take().unwrap();
    let output = std::thread::scope(|scope| {
        scope.spawn(move || {
            for message in messages {
                send_message(&mut input, message);
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
    std::iter::from_fn(|| read_message(&mut reader)).collect()
}

#[cfg(unix)]
#[test]
fn editor_merges_physical_buffers_with_linked_source_roots() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new("linked roots");
    let manifest = fixture.0.join("deps.edn");
    fs::write(&manifest, r#"{:paths ["linked"] :aliases {:test {}}}"#).unwrap();
    symlink("src", fixture.0.join("linked")).unwrap();
    let caller = fixture.0.join("src/caller.cljc");
    let provider = fixture.0.join("src/provider.clj");
    let app = "(ns caller (:require [provider :as p]))\n(defn label {:t/sig '[:-> string?]} [] (p/label))";
    let library = "(ns provider)\n(defn label {:t/sig '[:-> string?]} [] \"ok\")";
    fs::write(&caller, app).unwrap();
    fs::write(&provider, library).unwrap();
    let open = |path: &Path, text| {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":uri(path),"languageId":"clojure","version":1,"text":text}
        }})
    };
    let query = |id| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/definition","params":{
            "textDocument":{"uri":uri(&caller)},"position":{"line":1,"character":app.lines().nth(1).unwrap().find("p/label").unwrap()}
        }})
    };
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0),"capabilities":{}}}),
        open(&caller, app),
        open(&provider, library),
        query(2),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri(&provider),"version":2},"contentChanges":[{"text":library.replace("string?", "number?").replace("\"ok\"", "42")}]
        }}),
        json!({"jsonrpc":"2.0","method":"textDocument/didSave","params":{"textDocument":{"uri":uri(&manifest)}}}),
        query(3),
        json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":uri(&provider)}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    for id in [2, 3] {
        let target = messages.iter().find(|message| message["id"] == id).unwrap();
        assert_eq!(target["result"]["uri"], uri(&provider));
    }
    let caller_diagnostics: Vec<_> = messages
        .iter()
        .filter(|message| message["params"]["uri"] == uri(&caller))
        .map(|message| &message["params"]["diagnostics"])
        .collect();
    assert_eq!(caller_diagnostics.len(), 3, "{messages:?}");
    assert_eq!(caller_diagnostics[0], &json!([]));
    assert_eq!(caller_diagnostics[1][0]["code"], "PNX2322");
    assert_eq!(caller_diagnostics[2], &json!([]));
    assert_eq!(fs::read_to_string(&provider).unwrap(), library);
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
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":3},
            "contentChanges":[{"text":"{:paths [] :paths [\"src\"]}"}]
        }}),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":4},
            "contentChanges":[{"text":"{:paths [\"src\"]}"}]
        }}),
        json!({"jsonrpc":"2.0","id":2,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    assert_eq!(responses.len(), 6, "{responses:?}");
    assert_eq!(responses[1]["params"]["diagnostics"][0]["code"], "PNX9000");
    assert_eq!(
        responses[1]["params"]["diagnostics"][0]["range"]["start"],
        json!({"line":0,"character":8})
    );
    assert_eq!(responses[2]["params"]["version"], 2);
    assert_eq!(responses[2]["params"]["diagnostics"], json!([]));
    let duplicate = &responses[3]["params"]["diagnostics"][0];
    assert_eq!(duplicate["code"], "PNX9000");
    assert_eq!(
        duplicate["range"],
        json!({
            "start":{"line":0,"character":11},"end":{"line":0,"character":17}
        })
    );
    assert_eq!(responses[4]["params"]["diagnostics"], json!([]));
}

#[test]
fn watched_source_changes_refresh_closed_files_and_preserve_overlays() {
    let fixture = Fixture::new("watched sources");
    let provider = fixture.0.join("src/shop.cljc");
    let caller = fixture.0.join("test/caller.cljc");
    let string_contract =
        "(ns shop)\n(defn title {:t/sig '[string? :-> string?]} [name] (str name))";
    let number_contract = string_contract.replace("string? :->", "number? :->");
    fs::write(&provider, string_contract).unwrap();
    let saved = "(ns shop-test (:require [shop :as p]))\n(defn label {:t/sig '[:-> string?]} [] (p/title \"Alice\"))";
    fs::write(&caller, saved).unwrap();
    let overlay = saved.replace("\"Alice\"", "42");

    let mut child = lsp_server();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut exchange = |message| {
        send_message(&mut input, &message);
        read_message(&mut output).expect("language server produced no message")
    };
    let initialized = exchange(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&fixture.0),"capabilities":{"workspace":{"didChangeWatchedFiles":{"dynamicRegistration":true}}}
        }}),
    );
    assert_eq!(initialized["id"], 1);
    let register = exchange(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
    assert_eq!(register["method"], "client/registerCapability");
    let watch = &register["params"]["registrations"][0];
    assert_eq!(watch["method"], "workspace/didChangeWatchedFiles");
    assert!(
        watch["registerOptions"]["watchers"]
            .as_array()
            .unwrap()
            .contains(&json!({"globPattern":"**/*.cljc"}))
    );

    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","id":register["id"],"result":null}),
    );
    let open = json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
        "textDocument":{"uri":uri(&caller),"languageId":"clojure","version":7,"text":overlay}
    }});
    send_message(&mut input, &open);
    let broken = read_message(&mut output).unwrap();
    assert_eq!(broken["params"]["diagnostics"][0]["code"], "PNX2322");

    let mut exchange = |message| {
        send_message(&mut input, &message);
        read_message(&mut output).expect("language server produced no message")
    };
    let event = |kind| {
        json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles","params":{
            "changes":[{"uri":uri(&provider),"type":kind}]
        }})
    };
    fs::write(&provider, &number_contract).unwrap();
    let fixed = exchange(event(2));
    assert_eq!(fixed["params"]["version"], 7);
    assert_eq!(fixed["params"]["diagnostics"], json!([]));
    fs::remove_file(&provider).unwrap();
    let removed = exchange(event(3));
    assert_eq!(removed["params"]["diagnostics"][0]["code"], "PNX9003");
    fs::write(&provider, string_contract).unwrap();
    let created = exchange(event(1));
    assert_eq!(created["params"]["diagnostics"][0]["code"], "PNX2322");
    assert_eq!(fs::read_to_string(&caller).unwrap(), saved);

    assert_eq!(
        exchange(json!({"jsonrpc":"2.0","id":2,"method":"shutdown"}))["id"],
        2
    );
    send_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
}

#[test]
fn invalid_project_manifest_recovers_on_save_without_losing_overlays() {
    let fixture = Fixture::new("manifest reload");
    let manifest = fixture.0.join("deps.edn");
    let valid_manifest = fs::read_to_string(&manifest).unwrap();
    let broken_manifest = "{:paths [\"src\"}";
    fs::write(&manifest, broken_manifest).unwrap();
    let provider = fixture.0.join("src/shop.clj");
    let caller = fixture.0.join("test/caller.cljc");
    fs::write(
        &provider,
        "(ns shop) (defn title {:t/sig '[string? :-> string?]} [name] name)",
    )
    .unwrap();
    let saved = "(ns shop-test (:require [shop :as p]))\n(defn label {:t/sig '[:-> string?]} [] (p/title \"Alice\"))";
    fs::write(&caller, saved).unwrap();
    let overlay = saved.replace("\"Alice\"", "42");

    let mut child = lsp_server();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut request = |message| {
        send_message(&mut input, &message);
        read_message(&mut output).expect("language server produced no message")
    };
    assert_eq!(
        request(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "rootUri":uri(&fixture.0),"capabilities":{}
            }})
        )["result"]["capabilities"]["textDocumentSync"]["save"]["includeText"],
        false
    );
    let log = request(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
    assert_eq!(log["method"], "window/logMessage");
    assert!(
        log["params"]["message"]
            .as_str()
            .unwrap()
            .contains("deps.edn")
    );
    let open = |path: &Path, language, source| {
        json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
                "uri":uri(path),"languageId":language,"version":1,"text":source
            }}
        })
    };
    let broken = request(open(&manifest, "edn", broken_manifest));
    assert_eq!(broken["params"]["diagnostics"][0]["code"], "PNX9000");
    assert_eq!(
        broken["params"]["diagnostics"][0]["range"],
        json!({
            "start":{"line":0,"character":14},"end":{"line":0,"character":15}
        })
    );
    let missing = request(open(&caller, "clojure", &overlay));
    assert_eq!(missing["params"]["diagnostics"][0]["code"], "PNX9003");

    fs::write(&manifest, &valid_manifest).unwrap();
    let fixed = request(
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri(&manifest),"version":2},"contentChanges":[{"text":valid_manifest}]
        }}),
    );
    assert_eq!(fixed["params"]["diagnostics"], json!([]));
    let save = || {
        json!({
            "jsonrpc":"2.0","method":"textDocument/didSave","params":{"textDocument":{"uri":uri(&manifest)}}
        })
    };
    let reloaded = request(save());
    assert_eq!(reloaded["params"]["uri"], uri(&caller));
    assert_eq!(reloaded["params"]["diagnostics"][0]["code"], "PNX2322");
    assert_eq!(fs::read_to_string(&caller).unwrap(), saved);

    fs::write(&manifest, broken_manifest).unwrap();
    assert_eq!(request(save())["method"], "window/logMessage");
    let target = request(
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/definition","params":{
            "textDocument":{"uri":uri(&caller)},
            "position":{"line":1,"character":overlay.lines().nth(1).unwrap().find("p/title").unwrap()}
        }}),
    );
    assert_eq!(target["result"]["uri"], uri(&provider));

    assert_eq!(
        request(json!({"jsonrpc":"2.0","id":3,"method":"shutdown"}))["result"],
        Value::Null
    );
    send_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
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
    uri_explicit_referral(&root);
    let checkout = Fixture::new("uri disk edits");
    copy_sources(&root, &checkout.0, Path::new("src"));
    copy_sources(&root, &checkout.0, Path::new("test"));
    fs::copy(root.join("deps.edn"), checkout.0.join("deps.edn")).unwrap();
    uri_untyped_nil(&checkout.0);
    uri_assertion_template(&checkout.0);
    uri_closed_provider_edit(&checkout.0);
}

fn uri_untyped_nil(root: &Path) {
    let relative = Path::new("src/lambdaisland/uri.cljc");
    let caller = root.join(relative);
    let source = fs::read_to_string(&caller).unwrap();
    let edited = source.replacen(
        "(if v (normalize/percent-decode (str/replace",
        "(if true (normalize/percent-decode (str/replace",
        1,
    );
    assert_ne!(edited, source);
    let line = 134;
    let character = edited
        .lines()
        .nth(line)
        .unwrap()
        .find("str/replace v")
        .unwrap()
        + "str/replace ".len();
    assert_overlay_finding(
        root,
        relative,
        &source,
        &edited,
        ("PNX2322", line, character, "Nil | String"),
    );
}

fn uri_assertion_template(root: &Path) {
    let relative = Path::new("test/lambdaisland/uri/normalize_test.cljc");
    let source = fs::read_to_string(root.join(relative)).unwrap();
    let edited = source.replacen("(n/percent-decode in)", "(n/percent-decoed in)", 1);
    let line = 47;
    let character = edited
        .lines()
        .nth(line)
        .unwrap()
        .find("n/percent-decoed")
        .unwrap();
    assert_overlay_finding(
        root,
        relative,
        &source,
        &edited,
        ("PNX2304", line, character, "n/percent-decoed"),
    );
}

fn assert_overlay_finding(
    root: &Path,
    relative: &Path,
    source: &str,
    edited: &str,
    (code, line, character, message): (&str, usize, usize, &str),
) {
    let caller = root.join(relative);
    let document = uri(&caller);
    let change = |version, text| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(root),"capabilities":{}}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":document,"languageId":"clojure","version":1,"text":source}
        }}),
        change(2, edited),
        change(3, source),
        json!({"jsonrpc":"2.0","id":2,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let diagnostics = messages[2]["params"]["diagnostics"].as_array().unwrap();
    let mismatch = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == code)
        .unwrap();
    assert_eq!(
        mismatch["range"]["start"],
        json!({"line":line,"character":character})
    );
    assert!(mismatch["message"].as_str().unwrap().contains(message));
    assert_eq!(
        messages[1]["params"]["diagnostics"],
        messages[3]["params"]["diagnostics"]
    );
    assert_eq!(fs::read_to_string(&caller).unwrap(), source);
    fs::write(&caller, edited).unwrap();
    assert_cli_diagnostics(root, relative, diagnostics);
    fs::write(&caller, source).unwrap();
}

fn uri_explicit_referral(root: &Path) {
    let caller = root.join("src/lambdaisland/uri/normalize.cljc");
    let provider = root.join("src/lambdaisland/uri/platform.clj");
    let source = fs::read_to_string(&caller).unwrap();
    let line = source.lines().nth(4).unwrap();
    let column = line.find("hex->byte").unwrap();
    let inside = source.lines().nth(53).unwrap().find("str-len").unwrap();
    let same_namespace = source
        .lines()
        .nth(56)
        .unwrap()
        .find("high-surrogate?")
        .unwrap();
    let edited = source.replacen("hex->byte", "hex->byet", 1);
    let document = uri(&caller);
    let definition = |id, line, character| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/definition","params":{
            "textDocument":{"uri":document},"position":{"line":line,"character":character}
        }})
    };
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(root),"capabilities":{}}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":document,"languageId":"clojure","version":1,"text":source}
        }}),
        definition(2, 4, column),
        definition(3, 53, inside),
        definition(4, 56, same_namespace),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":2},"contentChanges":[{"text":edited}]
        }}),
        json!({"jsonrpc":"2.0","id":5,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    assert_eq!(messages[2]["result"]["uri"], uri(&provider));
    assert_eq!(messages[2]["result"]["range"]["start"]["line"], 8);
    assert_eq!(messages[3]["result"]["uri"], uri(&provider));
    assert_eq!(messages[3]["result"]["range"]["start"]["line"], 19);
    assert_eq!(messages[4]["result"]["uri"], uri(&caller));
    assert_eq!(messages[4]["result"]["range"]["start"]["line"], 42);
    assert!(
        messages[5]["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["code"] == "PNX2304"
                    && diagnostic["range"]["start"] == json!({"line":4,"character":column})
            })
    );
    assert_eq!(fs::read_to_string(caller).unwrap(), source);
}

fn uri_closed_provider_edit(root: &Path) {
    let provider = root.join("src/lambdaisland/uri/normalize.cljc");
    let caller = root.join("src/lambdaisland/uri.cljc");
    let library = fs::read_to_string(&provider).unwrap();
    let source = fs::read_to_string(&caller).unwrap();
    let mut child = lsp_server();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut exchange = |message| {
        send_message(&mut input, &message);
        read_message(&mut output).expect("language server produced no message")
    };
    let initialized = exchange(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(root),"capabilities":{}
        }}),
    );
    assert_eq!(initialized["id"], 1);
    let initial = exchange(
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":uri(&caller),"languageId":"clojure","version":1,"text":source}
        }}),
    );
    let edit_event = || {
        json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles","params":{
            "changes":[{"uri":uri(&provider),"type":2}]
        }})
    };

    let renamed = library.replacen("(defn percent-decode", "(defn percent-decode-renamed", 1);
    assert_ne!(renamed, library);
    fs::write(&provider, renamed).unwrap();
    let affected = exchange(edit_event());
    assert_eq!(affected["params"]["uri"], uri(&caller));
    assert!(
        affected["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["code"] == "PNX2304" && diagnostic["range"]["start"]["line"] == 133
            })
    );
    fs::write(&provider, library).unwrap();
    let fixed = exchange(edit_event());
    assert_eq!(fixed["params"], initial["params"]);

    assert_eq!(
        exchange(json!({"jsonrpc":"2.0","id":2,"method":"shutdown"}))["id"],
        2
    );
    send_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
}

#[test]
#[ignore = "measures the pinned lambdaisland/uri checkout"]
fn uri_pilot_editor_benchmark() {
    let root = PathBuf::from(std::env::var_os("PARENOX_URI_PILOT").expect("set PARENOX_URI_PILOT"));
    let binary = std::env::var_os("PARENOX_LSP_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_parenox")));
    let caller = root.join("src/lambdaisland/uri.cljc");
    let provider = root.join("src/lambdaisland/uri/normalize.cljc");
    let document = uri(&caller);
    let source = fs::read_to_string(&caller).unwrap();
    let correct = "normalize/percent-decode";
    let broken = source.replacen(correct, "normalize/percent-decoe", 1);
    let line = source.lines().nth(134).unwrap();
    let character = line.find(correct).unwrap();
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "rootUri":uri(&root),"capabilities":{}
    }});

    let started = Instant::now();
    let mut child = lsp_server_with(&binary);
    let spawn_us = started.elapsed().as_micros();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut exchange = |message: &Value| {
        let started = Instant::now();
        send_message(&mut input, message);
        let response = read_message(&mut output).expect("language server produced no message");
        (response, started.elapsed().as_micros())
    };
    let (initialized, initialize_round_trip_us) = exchange(&initialize);
    let startup_us = started.elapsed().as_micros();
    assert_eq!(initialized["result"]["serverInfo"]["name"], "parenox");
    let (opened, open_us) = exchange(
        &json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":document,"languageId":"clojure","version":1,"text":source}
        }}),
    );
    assert_eq!(opened["params"]["uri"], document);

    let mut edits = Vec::with_capacity(200);
    let mut hovers = Vec::with_capacity(200);
    let mut definitions = Vec::with_capacity(200);
    for version in 2..202 {
        let typo = version % 2 == 0;
        let change = json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},
            "contentChanges":[{"text":if typo { &broken } else { &source }}]
        }});
        let (published, elapsed) = exchange(&change);
        assert_eq!(published["params"]["version"], version);
        let bad_call = published["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "PNX2304");
        assert_eq!(bad_call, typo, "{published}");
        edits.push(elapsed);
        let query = |method, id| {
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":{
                "textDocument":{"uri":document},"position":{"line":134,"character":character}
            }})
        };
        let (hover, elapsed) = exchange(&query("textDocument/hover", 2 * version));
        assert!(
            hover["result"]["contents"]["value"]
                .as_str()
                .unwrap()
                .contains("lambdaisland.uri.normalize/percent-decode")
        );
        hovers.push(elapsed);
        let (definition, elapsed) = exchange(&query("textDocument/definition", 2 * version + 1));
        assert_eq!(definition["result"]["uri"], uri(&provider));
        assert_eq!(definition["result"]["range"]["start"]["line"], 81);
        definitions.push(elapsed);
    }

    let rss_kib = resident_kib(child.id());
    assert_eq!(
        exchange(&json!({"jsonrpc":"2.0","id":500,"method":"shutdown"})).0["result"],
        Value::Null
    );
    send_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
    let mut fresh_initializations = Vec::with_capacity(10);
    for _ in 0..10 {
        let started = Instant::now();
        let mut child = lsp_server_with(&binary);
        let mut input = child.stdin.take().unwrap();
        send_message(&mut input, &initialize);
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let ready = read_message(&mut output).unwrap();
        fresh_initializations.push(started.elapsed().as_micros());
        assert_eq!(ready["result"]["serverInfo"]["name"], "parenox");
        drop(input);
        assert!(child.wait().unwrap().success());
    }
    println!(
        "{}",
        json!({
            "binary":binary,
            "samples":200,
            "initialize_us":startup_us,
            "fresh_initialize_us":latencies(fresh_initializations),
            "spawn_us":spawn_us,
            "initialize_round_trip_us":initialize_round_trip_us,
            "first_open_us":open_us,
            "warm_edit_us":latencies(edits),
            "hover_us":latencies(hovers),
            "definition_us":latencies(definitions),
            "rss_kib_after_edits":rss_kib
        })
    );
}

fn latencies(mut samples: Vec<u128>) -> Value {
    samples.sort_unstable();
    let count = samples.len();
    json!({"p50":samples[(count - 1) / 2],"p95":samples[(95 * count).div_ceil(100) - 1],"max":samples[count - 1]})
}

#[test]
#[ignore = "requires the pinned metosin/malli checkout"]
fn malli_pilot_editor_benchmark() {
    let root =
        PathBuf::from(std::env::var_os("PARENOX_MALLI_PILOT").expect("set PARENOX_MALLI_PILOT"));
    let binary = std::env::var_os("PARENOX_LSP_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_parenox")));
    let caller = root.join("test/malli/destructure_test.cljc");
    let source = fs::read_to_string(&caller).unwrap();
    let edited = source.replacen(
        r#""map destructuring""#,
        r#""map destructuring example""#,
        1,
    );
    assert_ne!(edited, source);
    let document = uri(&caller);
    let started = Instant::now();
    let mut child = lsp_server_with(&binary);
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut exchange = |message: &Value| {
        let started = Instant::now();
        send_message(&mut input, message);
        let response = read_message(&mut output).expect("language server produced no message");
        (response, started.elapsed().as_micros())
    };
    let (initialize, _) = exchange(
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&root),"capabilities":{}
        }}),
    );
    let initialize_us = started.elapsed().as_micros();
    assert_eq!(initialize["result"]["serverInfo"]["name"], "parenox");
    let (opened, open_us) = exchange(
        &json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":document,"languageId":"clojure","version":1,"text":source}
        }}),
    );
    let base = opened["params"]["diagnostics"].as_array().unwrap();
    assert!(!base.is_empty());
    let mut edits = Vec::with_capacity(100);
    for version in 2..102 {
        let (published, elapsed) = exchange(
            &json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
                "textDocument":{"uri":document,"version":version},
                "contentChanges":[{"text":if version % 2 == 0 { &edited } else { &source }}]
            }}),
        );
        assert_eq!(published["params"]["version"], version);
        let diagnostics = published["params"]["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), base.len());
        assert!(base.iter().zip(diagnostics).all(|(old, new)| {
            ["code", "severity", "message"]
                .iter()
                .all(|key| old[*key] == new[*key])
        }));
        edits.push(elapsed);
    }
    let rss_kib = resident_kib(child.id());
    assert_eq!(
        exchange(&json!({"jsonrpc":"2.0","id":102,"method":"shutdown"})).0["result"],
        Value::Null
    );
    send_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
    println!(
        "{}",
        json!({
            "binary": binary,
            "samples": edits.len(),
            "diagnostics_per_edit": base.len(),
            "initialize_us": initialize_us,
            "first_open_us": open_us,
            "warm_edit_us": latencies(edits),
            "rss_kib_after_edits": rss_kib
        })
    );
}

fn resident_kib(pid: u32) -> Option<u32> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    std::str::from_utf8(&output.stdout)
        .ok()?
        .trim()
        .parse()
        .ok()
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
    assert_cli_diagnostics(&fixture.0, relative, diagnostics);
}

fn assert_cli_diagnostics(root: &Path, relative: &Path, diagnostics: &[Value]) {
    let checked = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .current_dir(root)
        .args([
            "check",
            "--alias",
            "test",
            "--allow-incomplete",
            "--format",
            "json",
        ])
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
            lsp["severity"],
            if cli["severity"] == "warning" { 2 } else { 1 }
        );
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

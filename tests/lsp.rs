use std::fs;
use std::io::{BufRead, BufReader, Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

use serde_json::{Value, json};

#[path = "lsp/clojuredocs.rs"]
mod clojuredocs;
#[path = "lsp/collections.rs"]
mod collections;
#[path = "lsp/cond_to_if.rs"]
mod cond_to_if;
#[path = "lsp/cycle_privacy.rs"]
mod cycle_privacy;
#[path = "lsp/formatting.rs"]
mod formatting;
#[path = "lsp/get_in_all.rs"]
mod get_in_all;
#[path = "lsp/get_in_less.rs"]
mod get_in_less;
#[path = "lsp/get_in_none.rs"]
mod get_in_none;
#[path = "lsp/hover_docs.rs"]
mod hover_docs;
#[path = "lsp/if_to_cond.rs"]
mod if_to_cond;
#[path = "lsp/inspection.rs"]
mod inspection;
#[path = "lsp/jar_definition.rs"]
mod jar_definition;
#[path = "lsp/java_completion.rs"]
mod java_completion;
#[path = "lsp/java_definition.rs"]
mod java_definition;
#[path = "lsp/java_type_hints.rs"]
mod java_type_hints;
#[path = "lsp/keyword_refactor.rs"]
mod keyword_refactor;
#[path = "lsp/literal_require_definition.rs"]
mod literal_require_definition;
#[path = "lsp/local_hover.rs"]
mod local_hover;
#[path = "lsp/medley_context.rs"]
mod medley_context;
#[path = "lsp/namespace_definition.rs"]
mod namespace_definition;
#[path = "lsp/protocol_implementations.rs"]
mod protocol_implementations;
#[path = "lsp/thread_first.rs"]
mod thread_first;
#[path = "lsp/thread_refactoring.rs"]
mod thread_refactoring;
#[path = "lsp/untyped_externs.rs"]
mod untyped_externs;
#[path = "lsp/watch_diagnostics.rs"]
mod watch_diagnostics;

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
    lsp_server_args(binary, ["lsp", "--alias", "test"])
}

fn lsp_server_args<const N: usize>(binary: &Path, args: [&str; N]) -> Child {
    Command::new(binary)
        .args(args)
        .arg("--clojuredocs-offline")
        .env(
            "XDG_CACHE_HOME",
            std::env::temp_dir().join(format!("parenox-lsp-empty-cache-{}", std::process::id())),
        )
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
    transcript_with(lsp_server(), messages)
}

fn transcript_with(mut child: Child, messages: &[Value]) -> Vec<Value> {
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

#[test]
#[ignore = "requires a Clojure LSP executable"]
fn compares_advertised_lsp_capabilities() {
    use std::collections::BTreeSet;

    let fixture = Fixture::new("capability comparison");
    let reference =
        std::env::var_os("PARENOX_REFERENCE_LSP").unwrap_or_else(|| "clojure-lsp".into());
    let candidate = std::env::var_os("PARENOX_LSP_CANDIDATE")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_parenox").into());
    let capabilities = |mut command: Command| {
        use parenox_lsp::protocol;
        let mut child = command
            .current_dir(&fixture.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut message = Vec::new();
        let mut exchange = |id, method, params| {
            protocol::write_message(
                &mut input,
                &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            )
            .unwrap();
            loop {
                assert!(protocol::read_message(&mut output, &mut message).unwrap());
                let response: Value = serde_json::from_slice(&message).unwrap();
                if response["id"] == id && response.get("method").is_none() {
                    assert!(response["error"].is_null(), "{response}");
                    break response["result"].clone();
                }
                if let (Some(id), Some(_)) = (response.get("id"), response.get("method")) {
                    protocol::write_message(
                        &mut input,
                        &json!({"jsonrpc":"2.0","id":id,"result":null}),
                    )
                    .unwrap();
                }
            }
        };
        let initialized = exchange(
            1,
            "initialize",
            json!({
                "rootUri":uri(&fixture.0),
                "capabilities":{
                    "workspace":{"applyEdit":true,"fileOperations":{"willRename":true,"didRename":true}},
                    "textDocument":{"rename":{"prepareSupport":true},"codeAction":{"resolveSupport":{"properties":["edit"]}}}
                }
            }),
        );
        exchange(2, "shutdown", Value::Null);
        protocol::write_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"})).unwrap();
        drop(input);
        assert!(child.wait().unwrap().success());
        initialized["capabilities"].clone()
    };
    let mut reference = Command::new(reference);
    reference.arg("listen");
    let mut candidate = Command::new(candidate);
    candidate.args(["lsp", "--clojuredocs-offline"]);
    let reference = capabilities(reference);
    let candidate = capabilities(candidate);

    let features = |capabilities: &Value| {
        let mut features = BTreeSet::new();
        for (name, value) in capabilities.as_object().expect("server capabilities") {
            if name.ends_with("Provider") && !matches!(value, Value::Null | Value::Bool(false)) {
                features.insert(name.clone());
            }
        }
        for path in ["/experimental", "/workspace/fileOperations"] {
            if let Some(options) = capabilities.pointer(path).and_then(Value::as_object) {
                for (name, value) in options {
                    if !matches!(value, Value::Null | Value::Bool(false)) {
                        features.insert(format!("{path}/{name}"));
                    }
                }
            }
        }
        for path in [
            "/codeActionProvider/resolveProvider",
            "/codeLensProvider/resolveProvider",
            "/completionProvider/resolveProvider",
            "/renameProvider/prepareProvider",
            "/semanticTokensProvider/full",
            "/semanticTokensProvider/range",
        ] {
            if capabilities
                .pointer(path)
                .is_some_and(|value| !matches!(value, Value::Null | Value::Bool(false)))
            {
                features.insert(path.into());
            }
        }
        features
    };
    let commands = |capabilities: &Value| -> BTreeSet<String> {
        capabilities["executeCommandProvider"]["commands"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    let reference_features = features(&reference);
    let candidate_features = features(&candidate);
    let reference_commands = commands(&reference);
    let candidate_commands = commands(&candidate);
    let missing: Vec<_> = reference_features.difference(&candidate_features).collect();
    let missing_commands: Vec<_> = reference_commands.difference(&candidate_commands).collect();
    println!(
        "{}",
        json!({
            "reference_features":reference_features.len(),
            "matching_advertisements":reference_features.intersection(&candidate_features).count(),
            "missing_advertisements":missing,
            "missing_commands":missing_commands,
        })
    );
    if std::env::var("PARENOX_LSP_PARITY").as_deref() == Ok("strict") {
        assert!(
            missing.is_empty() && missing_commands.is_empty(),
            "LSP capability gaps"
        );
    }
}

#[test]
fn formats_unsaved_buffers_with_the_project_profile_and_leaves_invalid_text_alone() {
    let fixture = Fixture::new("formatting");
    let path = fixture.0.join("src/demo.clj");
    fs::write(
        fixture.0.join("parenox.edn"),
        r#"{:format {:indent-width 4
        :indent-rules {my/wrap :block} :insert-final-newline? false}}"#,
    )
    .unwrap();
    let saved = "(ns saved)";
    fs::write(&path, saved).unwrap();
    let file = uri(&path);
    let text = "(my/wrap  :label\r\n  (println :indent)\r\n    (str \"🦊\"  :ok))";
    let expected = "(my/wrap :label\r\n    (println :indent)\r\n    (str \"🦊\" :ok))";
    let query = |id| {
        json!({
            "jsonrpc":"2.0","id":id,"method":"textDocument/formatting",
            "params":{"textDocument":{"uri":file},"options":{"tabSize":8,"insertSpaces":false}}
        })
    };
    let change = |version, text| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":file,"version":1,"languageId":"clojure","text":text}
        }}),
        query(2),
        change(2, expected),
        query(3),
        change(3, "(my/wrap"),
        query(4),
        json!({"jsonrpc":"2.0","id":5,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| &messages.iter().find(|m| m["id"] == id).unwrap()["result"];
    assert_eq!(
        response(1)["capabilities"]["documentFormattingProvider"],
        true
    );
    assert_eq!(
        response(2),
        &json!([
            {"range":{"start":{"line":0,"character":9},"end":{"line":0,"character":10}},"newText":""},
            {"range":{"start":{"line":1,"character":2},"end":{"line":1,"character":2}},"newText":"  "},
            {"range":{"start":{"line":2,"character":14},"end":{"line":2,"character":15}},"newText":""}
        ])
    );
    assert_eq!(response(3), &json!([]));
    assert_eq!(response(4), &json!([]));
    assert_eq!(fs::read_to_string(path).unwrap(), saved);
}

#[test]
fn range_formatting_preserves_unselected_context_and_tracks_utf16() {
    let fixture = Fixture::new("range formatting");
    let path = fixture.0.join("src/demo.clj");
    fs::write(
        fixture.0.join("parenox.edn"),
        "{:format {:indent-width 4 :indent-rules {my/wrap :block}}}",
    )
    .unwrap();
    let saved = "(ns saved)";
    fs::write(&path, saved).unwrap();
    let file = uri(&path);
    let text = "   (my/wrap  :label\r\n  (println :indent)\r\n       (str \"🦊\"  :ok))\r\n(def   later  1)";
    let formatted = text
        .replace("  (println", "       (println")
        .replace("\"🦊\"  :ok", "\"🦊\" :ok");
    let query = |id, start, end| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/rangeFormatting","params":{
            "textDocument":{"uri":file},"range":{"start":start,"end":end},
            "options":{"tabSize":8,"insertSpaces":false}
        }})
    };
    let change = |version, text| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let pos = |line, character| json!({"line":line,"character":character});
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":file,"version":1,"languageId":"clojure","text":text}
        }}),
        query(2, pos(1, 3), pos(3, 0)),
        query(3, pos(2, 16), pos(2, 17)),
        query(4, pos(2, 14), pos(2, 17)), // Inside the emoji's UTF-16 surrogate pair.
        query(5, pos(3, 0), pos(1, 0)),
        change(2, &formatted),
        query(6, pos(1, 3), pos(3, 0)),
        query(7, pos(3, 4), pos(3, 4)),
        change(3, &format!("{formatted}\r\n(defn unfinished")),
        query(8, pos(1, 3), pos(3, 0)),
        json!({"jsonrpc":"2.0","id":9,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| &messages.iter().find(|m| m["id"] == id).unwrap()["result"];
    assert_eq!(
        response(1)["capabilities"]["documentRangeFormattingProvider"],
        true
    );
    let edit = json!({"range":{"start":pos(2, 17),"end":pos(2, 18)},"newText":""});
    assert_eq!(
        response(2),
        &json!([
            {"range":{"start":pos(1, 2),"end":pos(1, 2)},"newText":"     "},
            edit
        ])
    );
    assert_eq!(response(3), &json!([edit]));
    for id in [4, 5, 6, 8] {
        assert_eq!(response(id), &json!([]), "response {id}");
    }
    assert_eq!(
        response(7),
        &json!([{"range":{"start":pos(3, 5),"end":pos(3, 13)},"newText":"later"}])
    );
    assert_eq!(fs::read_to_string(path).unwrap(), saved);
}

#[test]
fn structural_navigation_folds_declarations_and_expands_each_selection() {
    let fixture = Fixture::new("structural navigation");
    let path = fixture.0.join("src/demo.clj");
    let file = uri(&path);
    let row = "  (vector \"🦀\" [x :tag]))";
    let source = format!("(ns demo)\n(defn first-fn [x]\n{row}\n(defn next-fn []\n  0)\n");
    fs::write(&path, &source).unwrap();
    let character = row[..row.find("[x").unwrap() + 1].encode_utf16().count();
    let tag = row[..row.find(":tag").unwrap()].encode_utf16().count();
    let position = |line, character| json!({"line":line,"character":character});
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&fixture.0),
            "capabilities":{"textDocument":{"foldingRange":{"lineFoldingOnly":true,"rangeLimit":1}}}
        }}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/foldingRange","params":{
            "textDocument":{"uri":file}
        }}),
        json!({"jsonrpc":"2.0","id":3,"method":"textDocument/selectionRange","params":{
            "textDocument":{"uri":file},"positions":[position(2,character),position(2,tag)]
        }}),
        json!({"jsonrpc":"2.0","id":4,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| &messages.iter().find(|message| message["id"] == id).unwrap()["result"];
    assert_eq!(response(1)["capabilities"]["foldingRangeProvider"], true);
    assert_eq!(response(1)["capabilities"]["selectionRangeProvider"], true);
    assert_eq!(
        response(2),
        &json!([{"startLine":1,"endLine":2,"kind":"region"}])
    );
    let ranges = response(3).as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(
        ranges[0]["range"],
        json!({"start":position(2,character),"end":position(2,character+1)})
    );
    assert_eq!(
        ranges[1]["range"],
        json!({"start":position(2,tag),"end":position(2,tag+4)})
    );
    let mut outer = &ranges[0];
    while let Some(parent) = outer.get("parent") {
        outer = parent;
    }
    assert_eq!(outer["range"]["start"], position(1, 0));
    assert_eq!(
        outer["range"]["end"],
        position(2, row.encode_utf16().count())
    );
}

#[test]
fn range_formatting_accepts_an_unsaved_cljs_buffer() {
    let fixture = Fixture::new("cljs formatting");
    let path = fixture.0.join("src/demo.cljs");
    let file = uri(&path);
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":file,"languageId":"clojurescript","version":1,"text":"(ns demo)\n(defn f []\n(+ 1 2))"}
        }}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/rangeFormatting","params":{
            "textDocument":{"uri":file},
            "range":{"start":{"line":2,"character":0},"end":{"line":2,"character":3}},
            "options":{"tabSize":2,"insertSpaces":true}
        }}),
        json!({"jsonrpc":"2.0","id":3,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = messages.iter().find(|message| message["id"] == 2).unwrap();
    assert_eq!(
        response["result"],
        json!([
            {"range":{"start":{"line":2,"character":0},"end":{"line":2,"character":0}},"newText":"  "}
        ])
    );
}

#[test]
fn code_lenses_resolve_reference_counts_in_closed_source_files() {
    let fixture = Fixture::new("reference lenses");
    let provider = fixture.0.join("src/provider.clj");
    let caller = fixture.0.join("src/caller.clj");
    let file = uri(&provider);
    fs::write(&provider, "(ns provider)\n(defn shared [x] x)\n").unwrap();
    fs::write(
        &caller,
        "(ns caller (:require [provider :as p]))\n(defn call [] (p/shared 1))\n",
    )
    .unwrap();
    let lens = json!({
        "range":{"start":{"line":1,"character":6},"end":{"line":1,"character":12}},
        "data":[file,2,7]
    });
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/codeLens","params":{
            "textDocument":{"uri":file}
        }}),
        json!({"jsonrpc":"2.0","id":3,"method":"codeLens/resolve","params":lens}),
        json!({"jsonrpc":"2.0","id":4,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| &messages.iter().find(|message| message["id"] == id).unwrap()["result"];
    assert_eq!(
        response(1)["capabilities"]["codeLensProvider"]["resolveProvider"],
        true
    );
    assert_eq!(response(2), &json!([lens]));
    assert_eq!(
        response(3)["command"],
        json!({"title":"1 reference","command":"code-lens-references","arguments":[file,2,7]})
    );
}

#[test]
fn rename_refuses_a_target_from_dependency_only_sources() {
    let fixture = Fixture::new("read only rename target");
    let source = fixture.0.join("src/caller.clj");
    let dependency = fixture.0.join("vendor");
    fs::create_dir(&dependency).unwrap();
    fs::write(
        dependency.join("provider.clj"),
        "(ns provider) (defn run [] nil)",
    )
    .unwrap();
    fs::write(
        fixture.0.join("parenox.edn"),
        "{:paths [\"src\"] :dependency-sources [\"vendor\"]}",
    )
    .unwrap();
    fs::write(
        &source,
        "(ns caller (:require [provider :as p]))\n(defn main [] (p/run))",
    )
    .unwrap();
    let params = json!({
        "textDocument":{"uri":uri(&source)},
        "position":{"line":1,"character":20}
    });
    let mut rename = params.clone();
    rename["newName"] = "invoke".into();
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/prepareRename","params":params}),
        json!({"jsonrpc":"2.0","id":3,"method":"textDocument/rename","params":rename}),
        json!({"jsonrpc":"2.0","id":4,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| messages.iter().find(|message| message["id"] == id).unwrap();
    assert!(response(2)["result"].is_null(), "{messages:#?}");
    assert_eq!(response(3)["error"]["code"], -32602, "{messages:#?}");
}

#[test]
fn go_to_implementation_finds_source_methods_through_an_alias() {
    let fixture = Fixture::new("multimethod navigation");
    let provider = fixture.0.join("src/provider.clj");
    let methods = fixture.0.join("src/methods.clj");
    fs::write(&provider, "(ns provider)\n(defmulti render identity)\n").unwrap();
    fs::write(
        &methods,
        "(ns methods (:require [provider :as p]))\n(defmethod p/render :a [_] :a)\n",
    )
    .unwrap();
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/implementation","params":{
            "textDocument":{"uri":uri(&provider)},"position":{"line":1,"character":12}
        }}),
        json!({"jsonrpc":"2.0","id":3,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| messages.iter().find(|message| message["id"] == id).unwrap();
    assert_eq!(
        response(1)["result"]["capabilities"]["implementationProvider"],
        true
    );
    assert_eq!(
        response(2)["result"],
        json!([{
            "uri":uri(&methods),
            "range":{"start":{"line":1,"character":11},"end":{"line":1,"character":19}}
        }])
    );
}

#[test]
fn semantic_tokens_follow_lexical_bindings_in_unsaved_edits() {
    let fixture = Fixture::new("semantic tokens");
    let path = fixture.0.join("src/demo.clj");
    let source = "(ns demo)\n(defn target [x] x)\n(defn call [x] (target x))";
    let edited = source.replace("[x] (target x)", "[target] (target 1)");
    fs::write(&path, source).unwrap();
    let document_uri = uri(&path);
    let query = |id, source: &str| {
        let call = source.lines().nth(2).unwrap();
        let start = call.rfind("target").unwrap();
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/semanticTokens/range","params":{
            "textDocument":{"uri":document_uri},
            "range":{"start":{"line":2,"character":start + 1},"end":{"line":2,"character":start + 3}}
        }})
    };
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document_uri,"version":1,"languageId":"clojure","text":source
        }}}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/semanticTokens/full","params":{
            "textDocument":{"uri":document_uri}
        }}),
        query(3, source),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document_uri,"version":2},"contentChanges":[{"text":edited}]
        }}),
        query(4, &edited),
        json!({"jsonrpc":"2.0","id":5,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| &messages.iter().find(|m| m["id"] == id).unwrap()["result"];
    let provider = &response(1)["capabilities"]["semanticTokensProvider"];
    assert_eq!(provider["full"], true);
    assert_eq!(provider["range"], true);
    assert_eq!(provider["legend"]["tokenTypes"][2], "function");
    assert_eq!(provider["legend"]["tokenTypes"][6], "variable");
    assert!(response(2)["data"].as_array().unwrap().len() > 5);
    let expected = |text: &str, kind| {
        let start = text.lines().nth(2).unwrap().rfind("target").unwrap();
        json!({"data":[2,start,6,kind,0]})
    };
    assert_eq!(response(3), &expected(source, 2));
    assert_eq!(response(4), &expected(&edited, 6));
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
fn completion_resolve_reads_source_docs_without_changing_the_edit() {
    let fixture = Fixture::new("completion resolve");
    fs::create_dir(fixture.0.join("src/lib")).unwrap();
    fs::write(
        fixture.0.join("src/lib/main.clj"),
        r#"(ns lib.main)
(defn greet "Display a caller ID." {:t/sig '[number? :-> string?]} [id] (str id))"#,
    )
    .unwrap();
    fs::write(
        fixture.0.join("src/lib/other.clj"),
        r#"(ns lib.other) (defn greet "Unrelated." [value] value)"#,
    )
    .unwrap();
    let app = fixture.0.join("src/app.clj");
    let document_uri = uri(&app);
    let source = "(ns app (:require [lib.main :as lib]))\n(lib/gre)";
    let edited = source.replace("lib.main", "lib.other");
    fs::write(&app, source).unwrap();
    let position = |character| json!({"line":1,"character":character});
    let original = json!({
        "label":"lib/greet","kind":6,
        "textEdit":{"range":{"start":position(1),"end":position(8)},"newText":"lib/greet"},
        "data":{"parenoxSource":{"uri":document_uri,"label":"lib/greet","namespace":"lib.main"}}
    });
    let resolve = |id, item: &Value| json!({"jsonrpc":"2.0","id":id,"method":"completionItem/resolve","params":item});
    let unknown = json!({"label":"unknown","data":{"otherServer":true}});
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document_uri,"version":1,"languageId":"clojure","text":source
        }}}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/completion","params":{
            "textDocument":{"uri":document_uri},"position":position(8)
        }}),
        resolve(3, &original),
        resolve(4, &unknown),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document_uri,"version":2},"contentChanges":[{"text":edited}]
        }}),
        resolve(5, &original),
        json!({"jsonrpc":"2.0","id":6,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| &messages.iter().find(|m| m["id"] == id).unwrap()["result"];
    assert_eq!(
        response(1)["capabilities"]["completionProvider"]["resolveProvider"],
        true
    );
    assert_eq!(response(2)["items"], json!([original]));
    let mut resolved = response(3).clone();
    let documentation = resolved
        .as_object_mut()
        .unwrap()
        .remove("documentation")
        .unwrap();
    assert_eq!(resolved, original);
    assert_eq!(documentation["kind"], "plaintext");
    let doc = documentation["value"].as_str().unwrap();
    for fact in [
        "lib.main/greet",
        "(greet [id])",
        "Display a caller ID.",
        "[Number -> String]",
    ] {
        assert!(doc.contains(fact), "{doc}");
    }
    assert_eq!(response(4), &unknown);
    assert_eq!(response(5), &original);
    assert_eq!(fs::read_to_string(app).unwrap(), source);
}

#[test]
fn completes_incomplete_forms_project_vars_and_specs_from_closed_sources() {
    let fixture = Fixture::new("completion");
    let path = fixture.0.join("src/app.clj");
    let file = uri(&path);
    fs::write(
        fixture.0.join("src/api.clj"),
        "(ns lib.api (:require [clojure.spec.alpha :as s]))\n\
         (s/def :wire/id string?) (s/def ::title string?)\n\
         (defn render [x] x) (defn- render-private [x] x)\n",
    )
    .unwrap();
    let namespace = "(ns app (:require [lib.api :as lib]))\n";
    let incomplete = format!("{namespace}(de");
    let source = |last_line| format!("{namespace}{last_line}");
    let change = |version, text| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let query = |id, character| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/completion","params":{
            "textDocument":{"uri":file},"position":{"line":1,"character":character}
        }})
    };
    let alias_line = "(defn f [] (do \"🦊\" (lib/re";
    let keyword_line = "(defn f {:t/sig '[:-> :wire/i";
    let relative_line = "(defn f {:t/sig '[:-> ::lib/ti";
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&fixture.0),"capabilities":{"textDocument":{"completion":{"completionItem":{"snippetSupport":true}}}}
        }}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":file,"version":1,"languageId":"clojure","text":incomplete
        }}}),
        query(2, 3),
        change(2, source("(de unchanged [] nil)")),
        query(3, 3),
        change(3, source(alias_line)),
        query(4, alias_line.encode_utf16().count()),
        change(4, source(keyword_line)),
        query(5, keyword_line.len()),
        change(5, source(relative_line)),
        query(6, relative_line.len()),
        change(6, source(";; (de")),
        query(7, 6),
        change(7, source("(println \"de\")")),
        query(8, 12),
        json!({"jsonrpc":"2.0","id":9,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| &messages.iter().find(|m| m["id"] == id).unwrap()["result"];
    let find_item = |id, label| {
        response(id)["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["label"] == label)
            .unwrap()
    };
    assert!(response(1)["capabilities"]["completionProvider"].is_object());
    assert_eq!(find_item(2, "defn")["insertTextFormat"], 2);
    assert_eq!(
        find_item(2, "defn")["textEdit"],
        json!({
            "range":{"start":{"line":1,"character":1},"end":{"line":1,"character":3}},
            "newText":"defn ${1:name} [${2:args}] $0)"
        })
    );
    assert_eq!(find_item(3, "defn")["textEdit"]["newText"], "defn");
    assert_eq!(response(4)["items"].as_array().unwrap().len(), 1);
    let alias_end = alias_line.encode_utf16().count();
    assert_eq!(
        find_item(4, "lib/render")["textEdit"],
        json!({
            "range":{"start":{"line":1,"character":alias_end-6},"end":{"line":1,"character":alias_end}},
            "newText":"lib/render"
        })
    );
    assert_eq!(find_item(5, ":wire/id")["detail"], "Project spec");
    assert_eq!(
        find_item(6, "::lib/title")["textEdit"]["newText"],
        "::lib/title"
    );
    assert!(response(7).is_null());
    assert!(response(8).is_null());
}

#[test]
fn completes_public_jvm_vars_from_dependency_jars() {
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    let fixture = Fixture::new("jar completion");
    let jar = fixture.0.join("m2/vendor/tools/1.2.3/tools-1.2.3.jar");
    fs::create_dir_all(jar.parent().unwrap()).unwrap();
    let mut zip = ZipWriter::new(fs::File::create(&jar).unwrap());
    zip.start_file(
        "vendor/tools.cljc",
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
    )
    .unwrap();
    write!(
        zip,
        "(ns vendor.tools)\n\
         (defn query [x] x)\n\
         (defn- query-private [x] x)\n\
         (defn ^:private query-hidden [x] x)\n\
         #?(:clj (def query-options {{}}) :cljs (defn query-browser [] nil))\n"
    )
    .unwrap();
    zip.finish().unwrap();
    fs::write(
        fixture.0.join("deps.edn"),
        r#"{:paths ["src"] :mvn/local-repo "m2"
            :aliases {:test {:extra-deps {vendor/tools {:mvn/version "1.2.3"}}}}}"#,
    )
    .unwrap();
    let classpath = fixture.0.join("test.classpath");
    fs::write(
        &classpath,
        std::env::join_paths([Path::new("src"), &jar])
            .unwrap()
            .as_encoded_bytes(),
    )
    .unwrap();
    let file = uri(&fixture.0.join("src/app.clj"));
    let line = "(defn app [] (do \"🦊\" (v/qu";
    let end = line.encode_utf16().count();
    let source = format!("(ns app (:require [vendor.tools :as v]))\n{line}");
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":file,"version":1,"languageId":"clojure","text":source
        }}}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/completion","params":{
            "textDocument":{"uri":file},"position":{"line":1,"character":end}
        }}),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":2},
            "contentChanges":[{"text":source.strip_suffix("qu").unwrap()}]
        }}),
        json!({"jsonrpc":"2.0","id":3,"method":"textDocument/completion","params":{
            "textDocument":{"uri":file},"position":{"line":1,"character":end-2}
        }}),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":3},
            "contentChanges":[{"text":"(ns app)\n(require '[vendor.tools :as v])\n(comment (v/qu)"}]
        }}),
        json!({"jsonrpc":"2.0","id":4,"method":"textDocument/completion","params":{
            "textDocument":{"uri":file},"position":{"line":2,"character":"(comment (v/qu".len()}
        }}),
        json!({"jsonrpc":"2.0","id":5,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ];
    let binary = Path::new(env!("CARGO_BIN_EXE_parenox"));
    for server in [
        lsp_server(),
        lsp_server_args(binary, ["lsp", "--classpath", classpath.to_str().unwrap()]),
    ] {
        let responses = transcript_with(server, &requests);
        let items =
            &responses.iter().find(|message| message["id"] == 2).unwrap()["result"]["items"];
        assert_eq!(items.as_array().unwrap().len(), 2, "{items}");
        assert_eq!(items[0]["label"], "v/query");
        assert_eq!(items[0]["kind"], 3);
        assert_eq!(items[1]["label"], "v/query-options");
        assert_eq!(items[1]["kind"], 6);
        let after_slash =
            &responses.iter().find(|message| message["id"] == 3).unwrap()["result"]["items"];
        assert_eq!(after_slash[0]["label"], "v/query");
        assert_eq!(after_slash[1]["label"], "v/query-options");
        let after_require =
            &responses.iter().find(|message| message["id"] == 4).unwrap()["result"]["items"];
        assert_eq!(after_require[0]["label"], "v/query");
        assert_eq!(after_require[1]["label"], "v/query-options");
        assert_eq!(
            items[0]["textEdit"],
            json!({"newText":"v/query","range":{
                "start":{"line":1,"character":end-4},"end":{"line":1,"character":end}
            }})
        );
    }

    fs::write(
        fixture.0.join("src/local.clj"),
        "(ns vendor.tools) (defn qu-local [] nil)",
    )
    .unwrap();
    let responses = transcript(&requests);
    let items = &responses.iter().find(|message| message["id"] == 2).unwrap()["result"]["items"];
    assert_eq!(items.as_array().unwrap().len(), 1, "{items}");
    assert_eq!(items[0]["label"], "v/qu-local");
    let items = &responses.iter().find(|message| message["id"] == 4).unwrap()["result"]["items"];
    assert_eq!(items.as_array().unwrap().len(), 1, "{items}");
    assert_eq!(items[0]["label"], "v/qu-local");
}

#[test]
fn generates_a_versioned_fdef_from_the_unsaved_function() {
    let fixture = Fixture::new("generate fdef");
    let path = fixture.0.join("src/demo.clj");
    let ns = "(ns demo (:require [clojure.spec.alpha :as spec]))";
    let saved = format!("{ns}\n(defn bump [n] (str n))\n");
    let unsaved = format!("{ns}\n(defn bump [n] (inc n)) ;; keep 🦊\n");
    let generated =
        "(spec/fdef bump\n  :args (spec/cat :n clojure.core/number?)\n  :ret clojure.core/number?)";
    let applied = format!("{}\n\n{generated}\n", unsaved.trim_end());
    fs::write(&path, &saved).unwrap();
    let file = uri(&path);
    let query = |id, only: Value| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/codeAction","params":{
            "textDocument":{"uri":file},
            "range":{"start":{"line":1,"character":7},"end":{"line":1,"character":7}},
            "context":{"diagnostics":[],"only":only}
        }})
    };
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&fixture.0),
            "capabilities":{"workspace":{"workspaceEdit":{"documentChanges":true}}}
        }}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":file,"version":3,"languageId":"clojure","text":unsaved}
        }}),
        query(2, json!(["refactor"])),
        query(3, json!(["quickfix"])),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":4},"contentChanges":[{"text":applied}]
        }}),
        query(4, Value::Null),
        json!({"jsonrpc":"2.0","id":5,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| &messages.iter().find(|m| m["id"] == id).unwrap()["result"];
    let action_kinds = response(1)["capabilities"]["codeActionProvider"]["codeActionKinds"]
        .as_array()
        .unwrap();
    assert!(action_kinds.contains(&json!("refactor")));
    assert!(action_kinds.contains(&json!("refactor.rewrite")));
    let position =
        json!({"line":1,"character":unsaved.lines().nth(1).unwrap().encode_utf16().count()});
    assert_eq!(
        &response(2)[0],
        &json!({"title":"Generate s/fdef","kind":"refactor.rewrite","edit":{
            "documentChanges":[{"textDocument":{"uri":file,"version":3},"edits":[{
                "range":{"start":position,"end":position},"newText":format!("\n\n{generated}")
            }]}]
        }})
    );
    assert_eq!(response(3), &json!([]));
    assert_eq!(response(4).as_array().unwrap().len(), 1);
    assert_eq!(response(4)[0]["title"], "Cycle privacy");
    let diagnostics = messages
        .iter()
        .find(|m| m["method"] == "textDocument/publishDiagnostics" && m["params"]["version"] == 4)
        .unwrap();
    assert_eq!(diagnostics["params"]["diagnostics"], json!([]));
    assert_eq!(fs::read_to_string(path).unwrap(), saved);
}

#[test]
fn adds_spec_require_only_for_active_contracts_and_comments_unknown_types() {
    let fixture = Fixture::new("draft fdef");
    let path = fixture.0.join("src/demo.clj");
    let source = "(ns demo)\n(defn label [] \"hello\")\n(defn render [x] (str x))\n(defn hidden {:t/sig '[:-> string?]} [] \"ok\")\n(defn multi ([] nil) ([x] x))\n(defn shadow [inc] (inc 1))\n";
    fs::write(&path, source).unwrap();
    let file = uri(&path);
    let query = |id, line| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/codeAction","params":{
            "textDocument":{"uri":file},"range":{"start":{"line":line,"character":7},"end":{"line":line,"character":7}},
            "context":{"diagnostics":[]}
        }})
    };
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":file,"version":1,"languageId":"clojure","text":source}
        }}),
        query(2, 1),
        query(3, 2),
        query(4, 3),
        query(5, 4),
        query(6, 5),
        json!({"jsonrpc":"2.0","id":7,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| &messages.iter().find(|m| m["id"] == id).unwrap()["result"];
    let edits = |id| {
        response(id)[0]["edit"]["changes"][&file]
            .as_array()
            .unwrap()
    };
    let active = edits(2);
    assert_eq!(active.len(), 2);
    assert_eq!(active[0]["newText"], "\n  (:require [clojure.spec.alpha])");
    assert_eq!(active[0]["range"]["start"], json!({"line":0,"character":8}));
    assert_eq!(
        active[1]["newText"],
        "\n\n(clojure.spec.alpha/fdef label\n  :args (clojure.spec.alpha/cat)\n  :ret clojure.core/string?)"
    );
    let draft = edits(3);
    assert_eq!(draft.len(), 1);
    let text = draft[0]["newText"].as_str().unwrap();
    assert!(
        text.lines()
            .filter(|line| !line.is_empty())
            .all(|line| line.starts_with(";; "))
    );
    assert!(text.contains(":x clojure.core/any?"));
    assert!(text.contains(":ret clojure.core/string?"));
    for id in [4, 5] {
        assert_eq!(response(id).as_array().unwrap().len(), 1);
        assert_eq!(response(id)[0]["title"], "Cycle privacy");
    }
    assert!(
        edits(6)[0]["newText"]
            .as_str()
            .unwrap()
            .contains(":ret clojure.core/any?")
    );
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
fn editor_isolates_a_second_checkout_from_the_configured_project() {
    let fixture = Fixture::new("isolated editor source");
    let caller = fixture.0.join("src/store.cljc");
    let provider = fixture.0.join("src/catalog.clj");
    let other = fixture.0.join("other-checkout/src/catalog.clj");
    let source = "(ns store (:require [catalog :as c]))\n(defn entry [] (c/label))";
    let library = "(ns catalog)\n(defn label {:t/sig '[:-> string?]} [] \"ok\")";
    let outside = "(ns catalog)\n(defn- label {:t/sig '[:-> number?]} [] 42)\n(defn display {:t/sig '[:-> string?]} [] (label))";
    fs::write(&caller, source).unwrap();
    fs::write(&provider, library).unwrap();
    fs::create_dir_all(other.parent().unwrap()).unwrap();
    fs::write(&other, outside).unwrap();
    let open = |path: &Path, text| {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":uri(path),"languageId":"clojure","version":1,"text":text}
        }})
    };
    let definition = |id, path: &Path, line, character| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/definition","params":{
            "textDocument":{"uri":uri(path)},"position":{"line":line,"character":character}
        }})
    };
    let edited = source.replace("c/label", "c/lable");
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0),"capabilities":{}}}),
        open(&caller, source),
        open(&other, outside),
        definition(
            2,
            &caller,
            1,
            source.lines().nth(1).unwrap().find("c/label").unwrap(),
        ),
        definition(
            3,
            &other,
            2,
            outside.lines().nth(2).unwrap().rfind("(label)").unwrap() + 1,
        ),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri(&caller),"version":2},"contentChanges":[{"text":edited}]
        }}),
        json!({"jsonrpc":"2.0","id":4,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    assert_eq!(messages.len(), 7, "{messages:?}");
    assert_eq!(messages[2]["params"]["uri"], uri(&other));
    let outside_codes: Vec<_> = messages[2]["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(outside_codes, ["PNX2322"]);
    assert_eq!(messages[3]["result"]["uri"], uri(&provider));
    assert_eq!(messages[4]["result"]["uri"], uri(&other));
    assert_eq!(messages[5]["params"]["uri"], uri(&caller));
    assert!(
        messages[5]["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| {
                d["code"] == "PNX2304" && d["message"].as_str().unwrap().contains("c/lable")
            })
    );
}

#[test]
fn nested_project_uses_closed_specs_and_rechecks_only_its_own_overlays() {
    let fixture = Fixture::new("nested project");
    let primary = fixture.0.join("src/primary.clj");
    let primary_source = "(ns primary (:require [clojure.spec.alpha :as s]))\n\
        (s/def :wire/id number?)\n(defn id {:t/sig '[:-> :wire/id]} [] 42)\n";
    fs::write(&primary, primary_source).unwrap();

    let nested = fixture.0.join("examples/namespace_specs");
    fs::create_dir_all(&nested).unwrap();
    let app_path = nested.join("app.clj");
    let handler_path = nested.join("handler.clj");
    let config_path = nested.join("config.clj");
    let app = include_str!("../examples/namespace_specs/app.clj");
    let handler = include_str!("../examples/namespace_specs/handler.clj");
    fs::write(&app_path, app).unwrap();
    fs::write(&handler_path, handler).unwrap();
    fs::write(
        &config_path,
        include_str!("../examples/namespace_specs/config.clj"),
    )
    .unwrap();
    fs::write(
        nested.join("parenox.edn"),
        include_str!("../examples/namespace_specs/parenox.edn"),
    )
    .unwrap();

    let mut child = lsp_server();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut exchange = |message| {
        send_message(&mut input, &message);
        read_message(&mut output).expect("language server produced no message")
    };
    let open = |path: &Path, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":uri(path),"languageId":"clojure","version":1,"text":text}
        }})
    };
    let request =
        |id, method, params| json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
    let expect_diagnostics = |message: Value, path: &Path, codes: &[&str]| {
        assert_eq!(message["method"], "textDocument/publishDiagnostics");
        assert_eq!(message["params"]["uri"], uri(path));
        let actual: Vec<_> = message["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| diagnostic["code"].as_str().unwrap())
            .collect();
        assert_eq!(actual, codes, "{message}");
    };

    let init = exchange(request(1, "initialize", json!({"rootUri":uri(&fixture.0)})));
    assert_eq!(init["id"], 1);
    expect_diagnostics(exchange(open(&primary, primary_source)), &primary, &[]);
    expect_diagnostics(exchange(open(&app_path, app)), &app_path, &[]);

    // The user's failing case should be a real type error, not a missing spec.
    let wrong = app.replace("\"req-123\"", "123");
    let changed = |version, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri(&app_path),"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let mismatch = exchange(changed(2, &wrong));
    assert!(
        mismatch["params"]["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("expected String")
    );
    expect_diagnostics(mismatch, &app_path, &["PNX2322"]);

    // Update a spec in a closed file through a watch notification. The unsaved
    // numeric return now matches the nested project, not the primary project.
    let changed_spec = handler.replace("::id string?", "::id number?");
    fs::write(&handler_path, &changed_spec).unwrap();
    expect_diagnostics(
        exchange(
            json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles",
        "params":{"changes":[{"uri":uri(&handler_path),"type":2}]}}),
        ),
        &app_path,
        &[],
    );
    expect_diagnostics(
        exchange(open(&handler_path, &changed_spec)),
        &handler_path,
        &[],
    );
    let line = changed_spec
        .lines()
        .position(|line| line.contains("config/request-key"))
        .unwrap();
    let character = changed_spec
        .lines()
        .nth(line)
        .unwrap()
        .find("config/request-key")
        .unwrap();
    let target = exchange(request(
        2,
        "textDocument/definition",
        json!({
            "textDocument":{"uri":uri(&handler_path)},"position":{"line":line,"character":character}
        }),
    ));
    assert_eq!(target["result"]["uri"], uri(&config_path));

    assert_eq!(exchange(request(3, "shutdown", Value::Null))["id"], 3);
    send_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
    assert_eq!(fs::read_to_string(app_path).unwrap(), app);
}

#[test]
fn editor_applies_ordered_utf16_edits_atomically() {
    let fixture = Fixture::new("incremental source");
    let caller = fixture.0.join("src/demo.cljc");
    let body = r#"(defn label {:t/sig '[:-> string?]} [] (do "🦀" (s/trim "ok")))"#;
    let source = format!("(ns demo (:require [clojure.string :as s]))\r\n{body}\r\n");
    fs::write(&caller, &source).unwrap();
    let document = uri(&caller);
    let crab = body[..body.find('🦀').unwrap()].encode_utf16().count();
    let original_arg = body[..body.find("\"ok\"").unwrap()].encode_utf16().count();
    let edited_arg = original_arg - 2;
    let ranged = |start, end, text| json!({"range":{"start":{"line":1,"character":start},"end":{"line":1,"character":end}},"text":text});
    let change = |version, changes| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":changes
        }})
    };
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0),"capabilities":{}}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":document,"languageId":"clojure","version":1,"text":source}
        }}),
        change(
            2,
            json!([
                ranged(crab, crab + 2, ""),
                ranged(edited_arg, edited_arg + 4, "42")
            ]),
        ),
        change(3, json!([{"text":source}, ranged(crab + 1, crab + 2, "")])),
        change(3, json!([ranged(edited_arg, edited_arg + 2, "\"fixed\"")])),
        change(2, json!([{"text":"(invalid stale text"}])),
        json!({"jsonrpc":"2.0","id":2,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    assert_eq!(messages.len(), 5, "{messages:?}");
    assert_eq!(messages[1]["params"]["diagnostics"], json!([]));
    let broken = &messages[2]["params"];
    assert_eq!(broken["version"], 2);
    assert_eq!(broken["diagnostics"][0]["code"], "PNX2322");
    assert_eq!(
        broken["diagnostics"][0]["range"],
        json!({
            "start":{"line":1,"character":edited_arg},"end":{"line":1,"character":edited_arg + 2}
        })
    );
    assert_eq!(messages[3]["params"]["version"], 3);
    assert_eq!(messages[3]["params"]["diagnostics"], json!([]));
    assert_eq!(fs::read_to_string(caller).unwrap(), source);
}

#[test]
fn project_tree_expands_source_roots_and_live_namespace_symbols() {
    let fixture = Fixture::new("project tree");
    let source = fixture.0.join("src/demo.clj");
    let tests = fixture.0.join("test/demo_test.clj");
    let dependencies = fixture.0.join("vendor");
    fs::create_dir(&dependencies).unwrap();
    fs::write(
        dependencies.join("utility.clj"),
        "(ns library.utility) (defn read-it [] nil)",
    )
    .unwrap();
    fs::write(
        fixture.0.join("parenox.edn"),
        r#"{:dependency-sources ["vendor"]}"#,
    )
    .unwrap();
    let saved = "(ns demo)\n(def state 0)\n(defn old-title [] \"old\")\n";
    let overlay = r#"(ns demo (:require [clojure.spec.alpha :as s]))
(def state 0)
(defn title [] "🦀") (def caption "caption")
(defprotocol Readable (read-out [self]))
(s/def ::item string?)
"#;
    fs::write(&source, saved).unwrap();
    fs::write(&tests, "(ns demo-test)").unwrap();
    let request = |id, params| {
        json!({
            "jsonrpc":"2.0","id":id,"method":"clojure/workspace/projectTree/nodes","params":params
        })
    };
    let namespace = json!({"name":"demo","type":5,"uri":uri(&source),"final":false});
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        request(2, Value::Null),
        request(3, json!({"name":"src","type":2,"final":false})),
        request(4, json!({"name":"test","type":2,"final":false})),
        request(
            5,
            json!({"name":"External dependencies","type":3,"final":false,"id":"external-dependencies"}),
        ),
        request(6, namespace.clone()),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":uri(&source),"languageId":"clojure","version":1,"text":overlay
        }}}),
        request(7, namespace.clone()),
        request(
            8,
            json!({"name":"lib.jar","type":4,"final":false,"uri":"jar:file:///tmp/lib.jar"}),
        ),
        request(9, json!({"name":"vendor","type":2,"final":false})),
        json!({"jsonrpc":"2.0","id":10,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| &messages.iter().find(|message| message["id"] == id).unwrap()["result"];
    assert_eq!(
        result(1)["capabilities"]["experimental"]["projectTree"],
        true
    );
    assert_eq!(result(2)["type"], 1);
    assert_eq!(
        result(2)["nodes"],
        json!([
            {"name":"src","type":2,"final":false},
            {"name":"test","type":2,"final":false},
            {"name":"External dependencies","type":3,"id":"external-dependencies","final":false}
        ])
    );
    assert_eq!(result(3)["nodes"], json!([namespace]));
    assert_eq!(
        result(4)["nodes"],
        json!([{"name":"demo-test","type":5,"uri":uri(&tests),"final":false}])
    );
    assert!(result(5)["nodes"].as_array().unwrap().iter().all(|node| {
        node["type"] == 4
            && node["uri"]
                .as_str()
                .is_some_and(|uri| uri.starts_with("jar:file:"))
            && node["uri"] == node["detail"]
    }));
    assert_eq!(result(6)["nodes"][1]["name"], "old-title");
    let symbols = result(7)["nodes"].as_array().unwrap();
    let labels: Vec<_> = symbols
        .iter()
        .map(|node| (node["name"].clone(), node["type"].clone()))
        .collect();
    assert_eq!(
        labels,
        [
            (json!("state"), json!(8)),
            (json!("title"), json!(7)),
            (json!("caption"), json!(8)),
            (json!("read-out"), json!(7)),
            (json!("Readable"), json!(9)),
            (json!("item"), json!(7)),
        ]
    );
    let line = overlay.lines().nth(2).unwrap();
    let caption = line[..line.find("caption").unwrap()].encode_utf16().count();
    assert_eq!(
        symbols[2]["range"],
        json!({"start":{"line":2,"character":caption},"end":{"line":2,"character":caption+7}})
    );
    assert!(symbols.iter().all(|node| node["final"] == true));
    assert!(result(8).is_null());
    assert!(result(9).is_null());
    assert_eq!(fs::read_to_string(&source).unwrap(), saved);
}

#[test]
fn declaration_links_to_local_import_sites() {
    let fixture = Fixture::new("declaration");
    let api = fixture.0.join("src/api.clj");
    let caller = fixture.0.join("src/caller.clj");
    let api_text = "(ns api)\n(defn read! [] nil)\n(defn local [] nil)\n";
    let caller_text = concat!(
        "(ns caller \"🦀\" (:require [api :as source :refer :all]))\n",
        "(defn read-all [] (do (source/read!) (local)))\n",
    );
    fs::write(&api, api_text).unwrap();
    fs::write(&caller, caller_text).unwrap();
    let range = |text: &str, line: usize, name: &str| {
        let row = text.lines().nth(line).unwrap();
        let column = row[..row.find(name).unwrap()].encode_utf16().count();
        json!({"start":{"line":line,"character":column},
               "end":{"line":line,"character":column+name.len()}})
    };
    let query = |id, method, symbol| {
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":{
            "textDocument":{"uri":uri(&caller)},"position":range(caller_text,1,symbol)["start"]
        }})
    };
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        query(2, "textDocument/declaration", "source/read!"),
        query(3, "textDocument/declaration", "local"),
        query(4, "textDocument/definition", "source/read!"),
        json!({"jsonrpc":"2.0","id":5,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| &messages.iter().find(|m| m["id"] == id).unwrap()["result"];
    assert_eq!(result(1)["capabilities"]["declarationProvider"], true);
    assert_eq!(
        result(2),
        &json!({"uri":uri(&caller),"range":range(caller_text,0,"source")})
    );
    assert_eq!(
        result(3),
        &json!({"uri":uri(&caller),"range":range(caller_text,0,"api")})
    );
    assert_eq!(
        result(4),
        &json!({"uri":uri(&api),"range":range(api_text,1,"read!")})
    );
}

#[test]
fn call_hierarchy_groups_source_calls_and_tracks_unsaved_edits() {
    let fixture = Fixture::new("call hierarchy");
    let provider = fixture.0.join("src/provider.clj");
    let caller = fixture.0.join("src/caller.clj");
    let provider_text = concat!(
        "(ns provider)\n",
        "(defn title [x] x)\n",
        "(defn wrap [] (title \"local\"))\n",
    );
    let caller_text = concat!(
        "(ns caller (:require [provider :as p :refer [title] :rename {title renamed-title}]))\n",
        "(defn main [] (str \"🦀\" (p/title \"one\") (-> \"two\" renamed-title)))\n",
        "(defn values [] [p/title '(p/title \"quoted\")])\n",
        "(defn shadowed [] (let [renamed-title identity] (renamed-title \"local\")))\n",
        "(defn opaque [] (mystery (p/title \"macro data\")))\n",
    );
    fs::write(&provider, provider_text).unwrap();
    fs::write(&caller, caller_text).unwrap();
    let site = |source: &str, line: usize, needle: &str| {
        let text = source.lines().nth(line).unwrap();
        let start = text[..text.find(needle).unwrap()].encode_utf16().count();
        json!({"start":{"line":line,"character":start},
               "end":{"line":line,"character":start + needle.encode_utf16().count()}})
    };
    let request =
        |id, method, params| json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
    let prepare = |id, path: &Path, range: &Value| {
        request(
            id,
            "textDocument/prepareCallHierarchy",
            json!({"textDocument":{"uri":uri(path)},"position":range["start"]}),
        )
    };
    let target_range = site(provider_text, 1, "title");
    let caller_range = site(caller_text, 1, "main");
    let direct = site(caller_text, 1, "p/title");
    let thread = site(caller_text, 1, "renamed-title");
    let target = json!({"name":"title","uri":uri(&provider), "selectionRange":target_range});
    let origin = json!({"name":"main","uri":uri(&caller), "selectionRange":caller_range});
    let changed = caller_text.replace("(p/title \"one\") ", "");
    let messages = transcript(&[
        request(1, "initialize", json!({"rootUri":uri(&fixture.0)})),
        prepare(2, &caller, &direct),
        request(3, "callHierarchy/incomingCalls", json!({"item":target})),
        request(4, "callHierarchy/outgoingCalls", json!({"item":origin})),
        prepare(5, &caller, &site(caller_text, 4, "p/title")),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":uri(&caller),"languageId":"clojure","version":1,"text":changed
        }}}),
        request(6, "callHierarchy/outgoingCalls", json!({"item":origin})),
        request(7, "shutdown", Value::Null),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| &messages.iter().find(|m| m["id"] == id).unwrap()["result"];
    assert_eq!(result(1)["capabilities"]["callHierarchyProvider"], true);
    let item = &result(2)[0];
    assert_eq!(item["name"], "title");
    assert_eq!(item["detail"], "provider");
    assert_eq!(item["uri"], uri(&provider));
    assert_eq!(item["selectionRange"], target_range);
    let incoming = result(3).as_array().unwrap();
    assert_eq!(incoming.len(), 2, "{incoming:#?}");
    assert_eq!(incoming[0]["from"]["name"], "main");
    assert_eq!(incoming[0]["fromRanges"], json!([direct, thread]));
    assert_eq!(incoming[1]["from"]["name"], "wrap");
    assert_eq!(
        incoming[1]["fromRanges"],
        json!([site(provider_text, 2, "title")])
    );
    assert_eq!(
        result(4),
        &json!([{"to":item,"fromRanges":[direct,thread]}])
    );
    assert!(result(5).is_null());
    let after = result(6).as_array().unwrap();
    assert_eq!(after.len(), 1, "{after:#?}");
    assert_eq!(after[0]["fromRanges"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read_to_string(&caller).unwrap(), caller_text);
}

#[test]
fn signature_help_tracks_source_arities_and_unsaved_argument_positions() {
    let fixture = Fixture::new("signature help");
    let provider = fixture.0.join("src/provider.clj");
    let caller = fixture.0.join("src/caller.clj");
    fs::write(
        &provider,
        concat!(
            "(ns provider)\n",
            "(defn run \"Run a command.\" ([id] id) ([id opts] id) ([id opts & more] id))\n",
            "(defn inner [{:keys [value]}] value)\n",
            "(defn defaults [] nil)\n",
        ),
    )
    .unwrap();
    let source = concat!(
        "(ns caller (:require [provider :as p :refer [run]]))\n",
        "(defn execute [] (p/run \"🦀\" (p/inner {:value 9}) 3 4))\n",
        "(run 10)\n",
        "(p/defaults  )\n",
        "(defn local [] (let [run identity] (run 100)))\n",
        "'(p/run 101)\n",
    );
    fs::write(&caller, source).unwrap();
    let query = |id, text: &str, line, needle| {
        let text = text.lines().nth(line).unwrap();
        let column = text[..text.find(needle).unwrap()].encode_utf16().count();
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/signatureHelp","params":{
            "textDocument":{"uri":uri(&caller)}, "position":{"line":line,"character":column}
        }})
    };
    let unfinished = format!("{source}(defn later [] (p/run 1 ");
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0),"capabilities":{}}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":uri(&caller),"languageId":"clojure","version":1,"text":source
        }}}),
        query(2, source, 1, "{:value"),
        query(3, source, 1, "4))"),
        query(4, source, 2, "10"),
        query(5, source, 3, " )"),
        query(6, source, 4, "100"),
        query(7, source, 5, "101"),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri(&caller),"version":2}, "contentChanges":[{"text":unfinished}]
        }}),
        json!({"jsonrpc":"2.0","id":8,"method":"textDocument/signatureHelp","params":{
            "textDocument":{"uri":uri(&caller)}, "position":{"line":6,"character":unfinished.lines().last().unwrap().len()}
        }}),
        json!({"jsonrpc":"2.0","id":9,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| {
        messages
            .iter()
            .find(|message| message["id"] == id)
            .unwrap()
            .get("result")
            .unwrap()
    };
    assert!(result(1)["capabilities"]["signatureHelpProvider"].is_object());
    assert_eq!(
        result(2),
        &json!({"signatures":[{"label":"(inner [{:keys [value]}])",
            "parameters":[{"label":"{:keys [value]}"}]}],"activeSignature":0,"activeParameter":0})
    );
    let signatures = json!([
        {"label":"(run [id])","parameters":[{"label":"id"}],"documentation":"Run a command."},
        {"label":"(run [id opts])","parameters":[{"label":"id"},{"label":"opts"}],"documentation":"Run a command."},
        {"label":"(run [id opts & more])","parameters":[{"label":"id"},{"label":"opts"},{"label":"& more"}],"documentation":"Run a command."}
    ]);
    assert_eq!(result(3)["signatures"], signatures);
    assert_eq!(result(3)["activeSignature"], 2);
    assert_eq!(result(3)["activeParameter"], 2);
    assert_eq!(result(4)["activeSignature"], 0);
    assert_eq!(result(4)["activeParameter"], 0);
    assert_eq!(result(5)["signatures"][0]["label"], "(defaults [])");
    assert!(result(5).get("activeParameter").is_none());
    assert!(result(6).is_null());
    assert!(result(7).is_null());
    assert_eq!(result(8)["signatures"], signatures);
    assert_eq!(result(8)["activeSignature"], 1);
    assert_eq!(result(8)["activeParameter"], 1);
    assert_eq!(fs::read_to_string(caller).unwrap(), source);
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
    assert_eq!(capabilities["textDocumentSync"]["change"], 2);
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
fn references_track_real_usages_in_closed_files_and_unsaved_buffers() {
    let fixture = Fixture::new("find references");
    let provider = fixture.0.join("src/provider.clj");
    let client = fixture.0.join("src/client.clj");
    let other = fixture.0.join("src/other.clj");
    let provider_source = concat!(
        "(ns provider)\n",
        "(defn title [x] x)\n",
        "(defn wrap [] (title \"local\"))\n",
        "(let [title 0] (defn captured [] title))\n",
    );
    let client_source = indoc_client();
    fs::write(&provider, provider_source).unwrap();
    fs::write(&client, client_source).unwrap();
    fs::write(&other, "(ns other) (defn title [x] x)").unwrap();
    let request =
        |id, method, params| json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
    let position = |source: &str, line: usize, needle: &str| {
        let text = source.lines().nth(line).unwrap();
        let start = text.find(needle).unwrap();
        json!({"line":line,"character":text[..start].encode_utf16().count()})
    };
    let reference = |id, path: &Path, pos, include_declaration| {
        request(
            id,
            "textDocument/references",
            json!({
                "textDocument":{"uri":uri(path)},"position":pos,"context":{"includeDeclaration":include_declaration}
            }),
        )
    };
    let open = |path: &Path, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":uri(path),"languageId":"clojure","version":1,"text":text
        }}})
    };
    let declaration = position(provider_source, 1, "title");
    let overlay = client_source.replace("(detail \"two\") ", "");
    let parameters = position(&overlay, 5, "detail");
    let messages = transcript(&[
        request(1, "initialize", json!({"rootUri":uri(&fixture.0)})),
        open(&provider, provider_source),
        reference(2, &provider, declaration.clone(), false),
        reference(3, &provider, declaration.clone(), true),
        open(&client, &overlay),
        reference(4, &provider, declaration, false),
        reference(5, &client, parameters.clone(), false),
        request(
            6,
            "textDocument/documentHighlight",
            json!({
                "textDocument":{"uri":uri(&client)},"position":parameters
            }),
        ),
        request(7, "shutdown", Value::Null),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| &messages.iter().find(|m| m["id"] == id).unwrap()["result"];
    assert_eq!(response(1)["capabilities"]["referencesProvider"], true);
    let initial = response(2).as_array().unwrap();
    assert_eq!(initial.len(), 8, "{initial:#?}");
    let from_client: Vec<_> = initial
        .iter()
        .filter(|m| m["uri"] == uri(&client))
        .collect();
    assert_eq!(from_client.len(), 7, "{from_client:#?}");
    let on_line = |found: &Value, line: usize| found["range"]["start"]["line"] == line;
    assert_eq!(from_client.iter().filter(|m| on_line(m, 1)).count(), 3); // :refer and both :rename names
    assert_eq!(from_client.iter().filter(|m| on_line(m, 3)).count(), 2); // only actual calls, not other/title
    assert_eq!(from_client.iter().filter(|m| on_line(m, 6)).count(), 2); // call and var literal in def value
    assert!(
        from_client
            .iter()
            .any(|m| m["range"]["start"] == position(client_source, 3, "p/title"))
    );
    assert_eq!(response(3).as_array().unwrap().len(), 9);
    assert!(
        response(3)
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["uri"] == uri(&provider) && on_line(m, 1))
    );
    assert_eq!(response(4).as_array().unwrap().len(), 7);
    let uses = response(5).as_array().unwrap();
    assert_eq!(uses.len(), 1, "{uses:#?}");
    assert_eq!(
        uses[0]["range"]["start"],
        position(&overlay, 5, "detail \"initial\"")
    );
    let highlights = response(6).as_array().unwrap();
    assert_eq!(highlights.len(), 2, "{highlights:#?}");
    assert_eq!(
        highlights[0]["range"]["start"],
        position(&overlay, 5, "detail")
    );
    assert_eq!(fs::read_to_string(client).unwrap(), client_source);

    fn indoc_client() -> &'static str {
        concat!(
            "(ns client\n",
            "  (:require [provider :as p :refer [title] :rename {title detail}] [other :as o]))\n",
            "(defn use-it []\n",
            "  (do \"🦀\" (p/title \"one\") (detail \"two\") (o/title \"foreign\")\n",
            "      '(p/title) (quote p/title) #_(p/title) (opaque (p/title \"data\"))))\n",
            "(defn shadow [detail] (let [detail (detail \"initial\")] (detail \"inner\")))\n",
            "(def result [(p/title 1) #'p/title])\n",
        )
    }
}

#[test]
fn symbols_cover_closed_project_files_and_outline_survives_an_incomplete_edit() {
    let fixture = Fixture::new("outline");
    let api = fixture.0.join("src/api.clj");
    let nested = fixture.0.join("samples/widget");
    let app = nested.join("app.clj");
    let implementation = nested.join("model.clj");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("parenox.edn"), r#"{:paths ["."]}"#).unwrap();
    fs::write(&app, "(ns widget.app)").unwrap();
    fs::write(
        &implementation,
        "(ns widget.model) (defn widget-entry [] 1)",
    )
    .unwrap();
    let source = concat!(
        "(ns demo.api (:require [clojure.spec.alpha :as s]))\n",
        "(s/def ::id string?)\n",
        "(defprotocol Readable\n",
        "  (read-out [x]))\n",
        "#_(defn not-present [])\n",
        "(defn ^String read-name []\n",
        "  \"hello\")\n",
    );
    fs::write(&api, source).unwrap();
    let overlay = format!(
        "{}(defn open-form [",
        source.replace("read-name", "read-current")
    );
    let request =
        |id, method, params| json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
    let open = |path: &Path, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":uri(path),"languageId":"clojure","version":1,"text":text
        }}})
    };
    let search = |id, query| request(id, "workspace/symbol", json!({"query":query}));
    let messages = transcript(&[
        request(
            1,
            "initialize",
            json!({"rootUri":uri(&fixture.0),"capabilities":{
                "textDocument":{"documentSymbol":{"hierarchicalDocumentSymbolSupport":true}}
            }}),
        ),
        search(2, "demo.api/read-name"),
        open(&api, &overlay),
        request(
            3,
            "textDocument/documentSymbol",
            json!({"textDocument":{"uri":uri(&api)}}),
        ),
        search(4, "read-name"),
        search(5, "read-current"),
        open(&app, "(ns widget.app)"),
        search(6, "widget-entry"),
        search(7, "not-present"),
        request(8, "shutdown", Value::Null),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| &messages.iter().find(|m| m["id"] == id).unwrap()["result"];
    assert_eq!(response(1)["capabilities"]["documentSymbolProvider"], true);
    assert_eq!(response(1)["capabilities"]["workspaceSymbolProvider"], true);
    assert_eq!(response(2)[0]["location"]["uri"], uri(&api));
    let found = response(3).as_array().unwrap();
    let names: Vec<_> = found
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["demo.api", "::id", "Readable", "read-current"]);
    assert_eq!(found[2]["children"][0]["name"], "read-out");
    assert_eq!(found[3]["kind"], 12);
    assert_eq!(
        found[3]["selectionRange"]["start"],
        json!({"line":5,"character":14})
    );
    assert_eq!(found[3]["range"]["end"]["line"], 6);
    assert_eq!(response(4), &json!([]));
    assert_eq!(response(5)[0]["name"], "read-current");
    assert_eq!(response(6)[0]["location"]["uri"], uri(&implementation));
    assert_eq!(response(7), &json!([]));
}

#[test]
fn outline_and_definition_recover_after_an_unmatched_delimiter() {
    let fixture = Fixture::new("reader recovery");
    let path = fixture.0.join("src/recovered.cljc");
    let document = uri(&path);
    let source = concat!(
        "(ns recovered)\n",
        ")\n",
        "#?(:clj (defn later [] 42) :cljs (defn browser-only [] 0))\n",
        "(defn use-later [] (later))\n",
    );
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&fixture.0),"capabilities":{
                "textDocument":{"documentSymbol":{"hierarchicalDocumentSymbolSupport":true}}
            }
        }}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"languageId":"clojure","version":1,"text":source
        }}}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/documentSymbol","params":{
            "textDocument":{"uri":document}
        }}),
        json!({"jsonrpc":"2.0","id":3,"method":"textDocument/definition","params":{
            "textDocument":{"uri":document},"position":{"line":3,"character":20}
        }}),
        json!({"jsonrpc":"2.0","id":4,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let diagnostics = &messages
        .iter()
        .find(|message| message["method"] == "textDocument/publishDiagnostics")
        .unwrap()["params"]["diagnostics"];
    assert_eq!(diagnostics[0]["code"], "PNX9000");
    assert_eq!(
        diagnostics[0]["range"]["start"],
        json!({"line":1,"character":0})
    );

    let result = |id| &messages.iter().find(|message| message["id"] == id).unwrap()["result"];
    let outline = result(2).as_array().unwrap();
    let names: Vec<_> = outline
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["recovered", "later", "use-later"]);
    let declaration = json!({"start":{"line":2,"character":14},"end":{"line":2,"character":19}});
    assert_eq!(outline[1]["selectionRange"], declaration);
    assert_eq!(*result(3), json!({"uri":document,"range":declaration}));

    let checked = parenox::Session::default().check(path.to_str().unwrap(), source);
    assert!(!checked.ok && !checked.complete);
    assert_eq!(checked.diagnostics[0].code, "PNX9000");
}

#[test]
#[ignore = "requires the pinned Clojure LSP checkout"]
fn clojure_lsp_pilot_navigation() {
    let pilot =
        PathBuf::from(std::env::var_os("PARENOX_LSP_PILOT").expect("set PARENOX_LSP_PILOT"));
    let pilot = pilot.canonicalize().unwrap();
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"));
    let provider = pilot.join("lib/src/clojure_lsp/shared.clj");
    let service = pilot.join("cli/src/clojure_lsp/server.clj");
    let tests = pilot.join("lib/test/clojure_lsp/shared_test.clj");
    let source = fs::read_to_string(&provider).unwrap();
    let name = "normalize-uri-from-client";
    let (line, text) = source
        .lines()
        .enumerate()
        .find(|(_, line)| line.starts_with("(defn normalize-uri-from-client "))
        .unwrap();
    let character = text.find(name).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args(["lsp", "--deps"])
        .arg(pilot.join("deps.edn"))
        .args(["--alias", "test", "--clojuredocs-offline"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut id = 0;
    let mut request = |method: &str, params: Value, input: &mut std::process::ChildStdin| {
        id += 1;
        let before = Instant::now();
        send_message(
            input,
            &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
        );
        loop {
            let message = read_message(&mut output).expect("unexpected EOF");
            if message["id"] == id {
                assert!(message["error"].is_null(), "{message}");
                break (message["result"].clone(), before.elapsed().as_micros());
            }
        }
    };
    let (initialized, _) = request("initialize", json!({"rootUri":uri(workspace)}), &mut input);
    assert_eq!(initialized["capabilities"]["referencesProvider"], true);
    let symbol_params = json!({"query":format!("clojure-lsp.shared/{name}")});
    let (first_symbols, cold_symbols) =
        request("workspace/symbol", symbol_params.clone(), &mut input);
    assert_eq!(first_symbols[0]["location"]["uri"], uri(&provider));
    assert_eq!(
        first_symbols[0]["location"]["range"]["start"],
        json!({"line":line,"character":character})
    );
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":uri(&provider),"languageId":"clojure","version":1,"text":source}
        }}),
    );
    let usage_params = json!({"textDocument":{"uri":uri(&provider)},
        "position":{"line":line,"character":character},"context":{"includeDeclaration":false}});
    let mut warm_symbols = Vec::new();
    let mut references = Vec::new();
    let service = uri(&service);
    let tests = uri(&tests);
    let mut found_count = 0;
    for _ in 0..25 {
        let (found, elapsed) = request("textDocument/references", usage_params.clone(), &mut input);
        let hits = found.as_array().unwrap();
        assert!(hits.iter().any(|hit| hit["uri"] == service), "{found}");
        assert!(hits.iter().any(|hit| hit["uri"] == tests), "{found}");
        found_count = hits.len();
        references.push(elapsed);
        let (symbols, elapsed) = request("workspace/symbol", symbol_params.clone(), &mut input);
        assert_eq!(symbols, first_symbols);
        warm_symbols.push(elapsed);
    }
    request("shutdown", Value::Null, &mut input);
    send_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
    eprintln!(
        "{}",
        json!({
            "hits":found_count, "cold_symbol_us":cold_symbols,
            "warm_symbol_us":latencies(warm_symbols), "references_us":latencies(references)
        })
    );
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
fn checker_diagnostics_match_cli_after_unsaved_portable_edit() {
    let fixture = Fixture::new("checker overlay");
    let path = fixture.0.join("src/portable.cljc");
    let saved = r#"(ns portable (:require [clojure.spec.alpha :as s]))
(defn label ([] "") ([x] x))
(s/fdef label :args (s/alt :empty (s/cat) :one (s/cat :x string?)) :ret string?)
(defn exercise {:t/sig '[:-> boolean?]} []
  #?(:clj (label "ok") :cljrs (label))
  (for [x [1]] (inc x))
  (.startsWith "hello" "he"))
"#;
    let edited = saved
        .replace("(label \"ok\")", "(label 42)")
        .replace("[x [1]]", "[x [\"bad\"]]")
        .replace("\"hello\" \"he\"", "\"hello\" nil");
    fs::write(&path, saved).unwrap();
    let file = uri(&path);
    let change = |version, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":file,"version":1,"languageId":"clojure","text":saved}
        }}),
        change(2, &edited),
        change(3, saved),
        json!({"jsonrpc":"2.0","id":2,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let diagnostics = |version| {
        messages
            .iter()
            .find(|m| {
                m["method"] == "textDocument/publishDiagnostics"
                    && m["params"]["version"] == version
            })
            .map(|m| m["params"]["diagnostics"].as_array().unwrap())
            .unwrap()
    };
    assert!(diagnostics(1).is_empty(), "{:?}", diagnostics(1));
    assert!(diagnostics(3).is_empty(), "{:?}", diagnostics(3));
    assert_eq!(fs::read_to_string(&path).unwrap(), saved);

    let cli_input = fixture.0.join("edited.cljc");
    fs::write(&cli_input, edited).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args(["check", "--format", "json"])
        .arg(cli_input)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let checked: Value = serde_json::from_slice(&output.stdout).unwrap();
    let cli = checked["diagnostics"].as_array().unwrap();
    let lsp = diagnostics(2);
    assert_eq!(cli.len(), 3, "{cli:?}");
    assert_eq!(cli.len(), lsp.len(), "{lsp:?}");
    for (cli, lsp) in cli.iter().zip(lsp) {
        assert_eq!(cli["code"], "PNX2322");
        assert_eq!(lsp["code"], cli["code"]);
        assert_eq!(lsp["message"], cli["message"]);
        assert_eq!(lsp["severity"], 1);
        for (lsp_pos, cli_pos) in [("start", "start"), ("end", "end")] {
            let point = &cli["span"][cli_pos];
            assert_eq!(
                lsp["range"][lsp_pos],
                json!({
                    "line":point["line"].as_u64().unwrap()-1,
                    "character":point["column"].as_u64().unwrap()-1
                })
            );
        }
    }
}

#[test]
fn follows_spec_references_to_closed_and_unsaved_declarations() {
    let fixture = Fixture::new("spec definition");
    let path = fixture.0.join("src/demo.cljc");
    let provider = fixture.0.join("src/wire.clj");
    let definitions = "(ns wire (:require [clojure.spec.alpha :as s]))\n(s/def ::value string?)\n";
    let row = "(s/def ::row (s/keys :req-un [::value]))\n";
    let saved = format!("{definitions}{row}");
    let overlay = format!("{definitions};; inserted in the editor\n{row}");
    let source = r#"(ns demo (:require [wire :as-alias w] [clojure.spec.alpha :as s]))
(defn checked-row {:t/sig '[map? :-> ::w/row]} [row]
  (if (s/valid? ::w/row row) row (throw (ex-info "invalid row" {}))))
"#;
    fs::write(&path, source).unwrap();
    fs::write(&provider, &saved).unwrap();
    let open = |path: &Path, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":uri(path),"version":1,"languageId":"clojure","text":text}
        }})
    };
    let jump = |id, line: usize| {
        let character = source.lines().nth(line).unwrap().find("::w/row").unwrap();
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/definition","params":{
            "textDocument":{"uri":uri(&path)},"position":{"line":line,"character":character}
        }})
    };
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        open(&path, source),
        jump(2, 1),
        jump(3, 2),
        open(&provider, &overlay),
        jump(4, 1),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri(&provider),"version":2},"contentChanges":[{"text":definitions}]
        }}),
        jump(5, 1),
        json!({"jsonrpc":"2.0","id":6,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| &messages.iter().find(|message| message["id"] == id).unwrap()["result"];
    let location = |line| {
        json!({
            "uri":uri(&provider),
            "range":{"start":{"line":line,"character":7},"end":{"line":line,"character":12}}
        })
    };
    let initial = messages
        .iter()
        .find(|m| {
            m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri(&path)
        })
        .unwrap();
    let diagnostics = initial["params"]["diagnostics"].as_array().unwrap();
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["code"] == "PNX9001"
            && diagnostic["message"]
                == "Cannot prove the named spec and its registrations are loaded"
    }));
    assert_eq!(*result(2), location(2));
    assert_eq!(*result(3), location(2));
    assert_eq!(*result(4), location(3));
    assert_eq!(*result(5), Value::Null);
    assert_eq!(fs::read_to_string(provider).unwrap(), saved);
}

#[test]
fn changing_an_open_dependency_spec_rechecks_rest_bindings_in_consumers() {
    let fixture = Fixture::new("dependency overlay");
    let provider = fixture.0.join("wire.clj");
    let path = fixture.0.join("src/demo.cljc");
    let dependency = "(ns wire (:require [clojure.spec.alpha :as s]))\n(s/def ::element number?)";
    let edited = dependency.replace("number?", "string?");
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
(defn bump {:t/sig '[(s/coll-of :wire/element :kind seq?) :-> number?]}
  [items]
  (if (empty? items) 0
    (loop [[x & xs] items]
      (if (empty? xs) (inc x) (recur xs)))))
"#;
    fs::write(
        fixture.0.join("parenox.edn"),
        r#"{:dependency-sources ["wire.clj"]}"#,
    )
    .unwrap();
    fs::write(&provider, dependency).unwrap();
    fs::write(&path, source).unwrap();
    let open = |path: &Path, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":uri(path),"version":1,"languageId":"clojure","text":text}
        }})
    };
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        open(&path, source),
        open(&provider, dependency),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri(&provider),"version":2},"contentChanges":[{"text":edited}]
        }}),
        json!({"jsonrpc":"2.0","id":2,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let published: Vec<_> = messages
        .iter()
        .filter(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri(&path)
        })
        .map(|message| &message["params"]["diagnostics"])
        .collect();
    assert_eq!(published.len(), 2, "{messages:?}");
    assert_eq!(*published[0], json!([]));
    assert_eq!(published[1].as_array().unwrap().len(), 1);
    let lsp = &published[1][0];
    assert_eq!(lsp["code"], "PNX2322");
    assert_eq!(lsp["range"]["start"], json!({"line":5,"character":27}));
    assert_eq!(fs::read_to_string(&provider).unwrap(), dependency);

    fs::write(&provider, edited).unwrap();
    let cli = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .current_dir(&fixture.0)
        .args(["check", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(cli.status.code(), Some(1));
    let checked: Value = serde_json::from_slice(&cli.stdout).unwrap();
    let consumer = checked["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == "src/demo.cljc")
        .unwrap();
    let findings = consumer["diagnostics"].as_array().unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["message"], lsp["message"]);
    assert_eq!(findings[0]["code"], lsp["code"]);
    assert_eq!(findings[0]["span"]["start"]["line"], 6);
    assert_eq!(findings[0]["span"]["start"]["column"], 28);
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
    let change = |version, before: &str, after: &str| {
        let before = before.lines().nth(line).unwrap();
        let after = after.lines().nth(line).unwrap();
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":[{
                "text":after,"range":{"start":{"line":line,"character":0},
                    "end":{"line":line,"character":before.encode_utf16().count()}}
            }]
        }})
    };
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(root),"capabilities":{}}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":document,"languageId":"clojure","version":1,"text":source}
        }}),
        change(2, source, edited),
        change(3, edited, source),
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
    let transport = std::env::var("PARENOX_LSP_TRANSPORT").unwrap_or_else(|_| "full".into());
    assert!(matches!(transport.as_str(), "full" | "range"));
    let selection = std::env::var("PARENOX_MALLI_DOCUMENT").unwrap_or_else(|_| "assertions".into());
    let (file, before, after) = match selection.as_str() {
        "assertions" => (
            "test/malli/destructure_test.cljc",
            r#""map destructuring""#,
            r#""map destructuring example""#,
        ),
        "core" => (
            "src/malli/core.cljc",
            "returns type of the schema",
            "returns type of the schema value",
        ),
        _ => panic!("invalid Malli benchmark document {selection}"),
    };
    let caller = root.join(file);
    let source = fs::read_to_string(&caller).unwrap();
    let prefix = &source[..source.find(before).unwrap()];
    let line = prefix.bytes().filter(|&byte| byte == b'\n').count();
    let character = prefix.rsplit('\n').next().unwrap().encode_utf16().count();
    let edited = source.replacen(before, after, 1);
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
    let mut first_edit_bytes = None;
    for version in 2..102 {
        let forward = version % 2 == 0;
        let change = if transport == "range" {
            let (old, new) = if forward {
                (before, after)
            } else {
                (after, before)
            };
            let end = character + old.encode_utf16().count();
            json!({"text":new,"range":{
                "start":{"line":line,"character":character},
                "end":{"line":line,"character":end}
            }})
        } else {
            json!({"text":if forward { &edited } else { &source }})
        };
        let notification = json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":[change]
        }});
        let (published, elapsed) = exchange(&notification);
        first_edit_bytes.get_or_insert_with(|| notification.to_string().len());
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
            "file": file,
            "transport": transport,
            "samples": edits.len(),
            "first_edit_json_bytes": first_edit_bytes,
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

#[test]
fn rename_tracks_cross_file_vars_and_lexical_bindings_without_capture() {
    let fixture = Fixture::new("safe rename");
    let provider = fixture.0.join("src/provider.clj");
    let caller = fixture.0.join("src/caller.clj");
    let provider_source = concat!(
        "(ns provider (:require [clojure.spec.alpha :as s]))\n",
        "(declare run)\n",
        "(s/fdef run :args (s/cat :x string?) :ret string?)\n",
        "(defn again [x] (run x))\n",
        "(defn run [x] x)\n",
    );
    let source = concat!(
        "(ns caller (:require [provider :as p :refer [run]]))\n",
        "(defn call [] [(p/run \"🦊\") (run \"yay\") (let [run 2] run)])\n",
        "(defn lexical [value]\n",
        "  [(let [value :shadow] value) (let [conflict 0] [conflict value]) 'value])\n",
        "(defn keyed [{:keys [total]}] total)\n",
        "(defn opaque [blocked] (unmodeled/macro blocked))\n",
    );
    fs::write(&provider, provider_source).unwrap();
    fs::write(&caller, source).unwrap();
    let pos = |source: &str, line: usize, needle: &str| {
        let line_text = source.lines().nth(line).unwrap();
        let start = line_text.find(needle).unwrap();
        json!({"line":line,"character":line_text[..start].encode_utf16().count()})
    };
    let query = |id, method, position, name: Option<&str>| {
        let mut params = json!({"textDocument":{"uri":uri(&caller)},"position":position});
        if let Some(name) = name {
            params["newName"] = json!(name);
        }
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
    };
    let qualified = pos(source, 1, "run \"🦊\"");
    let lexical = pos(source, 2, "value");
    let initial = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "rootUri":uri(&fixture.0),"capabilities":{"workspace":{"workspaceEdit":{"documentChanges":true}}}
    }});
    let open = json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
        "textDocument":{"uri":uri(&caller),"version":7,"languageId":"clojure","text":source}
    }});
    let messages = transcript(&[
        initial,
        open,
        query(2, "textDocument/prepareRename", qualified.clone(), None),
        query(3, "textDocument/rename", qualified, Some("invoke")),
        query(4, "textDocument/rename", lexical.clone(), Some("renamed")),
        query(5, "textDocument/rename", lexical.clone(), Some("conflict")),
        query(
            6,
            "textDocument/rename",
            lexical.clone(),
            Some("ns/invalid"),
        ),
        query(7, "textDocument/linkedEditingRange", lexical, None),
        query(
            8,
            "textDocument/prepareRename",
            pos(source, 4, "total"),
            None,
        ),
        query(
            9,
            "textDocument/prepareRename",
            pos(source, 5, "blocked"),
            None,
        ),
        json!({"jsonrpc":"2.0","id":10,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| messages.iter().find(|message| message["id"] == id).unwrap();
    assert_eq!(
        response(1)["result"]["capabilities"]["renameProvider"]["prepareProvider"],
        true
    );
    let range = |source: &str, line: usize, needle: &str, len: usize| {
        let start = pos(source, line, needle);
        json!({"start":start,"end":{"line":line,"character":start["character"].as_u64().unwrap() + len as u64}})
    };
    assert_eq!(
        response(2)["result"],
        json!({
            "placeholder":"run","range":range(source, 1, "run \"🦊\"", 3)
        })
    );
    let edit = |source, line, needle, len, new| json!({"range":range(source, line, needle, len),"newText":new});
    assert_eq!(
        response(3)["result"],
        json!({"documentChanges":[
            {"textDocument":{"uri":uri(&caller),"version":7},"edits":[
                edit(source, 0, "run", 3, "invoke"),
                edit(source, 1, "run \"🦊\"", 3, "invoke"),
                edit(source, 1, "run \"yay\"", 3, "invoke")
            ]},
            {"textDocument":{"uri":uri(&provider),"version":null},"edits":[
                edit(provider_source, 1, "run", 3, "invoke"),
            edit(provider_source, 2, "run", 3, "invoke"),
            edit(provider_source, 3, "run x", 3, "invoke"),
            edit(provider_source, 4, "run", 3, "invoke")
            ]}
        ]})
    );
    assert_eq!(
        response(4)["result"],
        json!({"documentChanges":[
            {"textDocument":{"uri":uri(&caller),"version":7},"edits":[
                edit(source, 2, "value", 5, "renamed"),
                edit(source, 3, "value])", 5, "renamed")
            ]}
        ]})
    );
    for id in [5, 6] {
        assert_eq!(response(id)["error"]["code"], -32602);
    }
    assert_eq!(
        response(7)["result"],
        json!({"ranges":[
            range(source, 2, "value", 5),
            range(source, 3, "value])", 5)
        ]})
    );
    assert!(response(8)["result"].is_null());
    assert!(response(9)["result"].is_null());
    assert_eq!(fs::read_to_string(&caller).unwrap(), source);
    assert_eq!(fs::read_to_string(&provider).unwrap(), provider_source);
}

#[test]
fn test_tree_notifications_follow_the_opted_in_buffer_and_resolved_test_scopes() {
    let fixture = Fixture::new("test tree");
    let path = fixture.0.join("src/example.clj");
    let file = uri(&path);
    let source = concat!(
        "(ns example (:require [clojure.test :as t :refer [deftest testing]]))\n",
        "(deftest smoke\n",
        "  (testing \"🦊 outer\" (t/testing \"inner\" 1))\n",
        "  (let [testing identity] (testing \"shadow\"))\n",
        "  '(testing \"quoted\"))\n",
        "(t/deftest empty-test)\n",
    );
    fs::write(&path, source).unwrap();
    let edited = source.replace("🦊 outer", "🦊 revised");
    let init = |enabled| {
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&fixture.0),"capabilities":{"experimental":{"testTree":enabled}}
        }})
    };
    let open = json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
        "uri":file,"languageId":"clojure","version":1,"text":source
    }}});
    let shutdown = json!({"jsonrpc":"2.0","id":8,"method":"shutdown"});
    let exit = json!({"jsonrpc":"2.0","method":"exit"});
    let messages = transcript(&[
        init(true),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        open.clone(),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":2},"contentChanges":[{"text":edited}]
        }}),
        shutdown.clone(),
        exit.clone(),
    ]);
    assert_eq!(
        messages[0]["result"]["capabilities"]["experimental"]["testTree"],
        true
    );
    let trees: Vec<_> = messages
        .iter()
        .filter(|message| message["method"] == "clojure/textDocument/testTree")
        .map(|message| &message["params"])
        .collect();
    assert!(trees.len() >= 2, "{messages:#?}");
    let first = trees
        .iter()
        .find(|params| params["tree"]["children"][0]["children"][0]["name"] == "🦊 outer")
        .unwrap();
    assert_eq!(first["uri"], file);
    let tree = &first["tree"];
    assert_eq!(tree["name"], "example");
    assert_eq!(tree["kind"], 1);
    assert_eq!(
        tree["nameRange"],
        json!({"start":{"line":0,"character":4},"end":{"line":0,"character":11}})
    );
    let test = &tree["children"][0];
    assert_eq!(test["name"], "smoke");
    assert_eq!(test["kind"], 2);
    assert_eq!(test["children"].as_array().unwrap().len(), 1);
    let outer = &test["children"][0];
    assert_eq!(outer["kind"], 3);
    assert_eq!(outer["children"][0]["name"], "inner");
    assert_eq!(outer["children"][0]["kind"], 3);
    assert!(outer["children"][0].get("children").is_none());
    let text = source.lines().nth(2).unwrap();
    let column = text[..text.find("t/testing").unwrap()]
        .encode_utf16()
        .count();
    assert_eq!(
        outer["children"][0]["nameRange"],
        json!({"start":{"line":2,"character":column},"end":{"line":2,"character":column+9}})
    );
    assert_eq!(tree["children"][1]["name"], "empty-test");
    assert_eq!(tree["children"][1]["children"], json!([]));
    assert_eq!(
        trees.last().unwrap()["tree"]["children"][0]["children"][0]["name"],
        "🦊 revised"
    );
    let silent = transcript(&[init(false), open, shutdown, exit]);
    assert!(
        silent
            .iter()
            .all(|message| message["method"] != "clojure/textDocument/testTree")
    );
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
fn file_move_plans_namespace_edits_and_reindexes_the_renamed_overlay() {
    let fixture = Fixture::new("namespace move");
    let package = fixture.0.join("src/demo");
    fs::create_dir_all(&package).unwrap();
    let old = package.join("old.cljc");
    let new = package.join("new_name.cljc");
    let caller = package.join("caller.cljc");
    let occupied = package.join("occupied.cljc");
    fs::write(&occupied, "(ns demo.occupied)").unwrap();
    let provider = "(ns demo.old)\n(defn show [x] x)\n";
    let overlay = format!("{provider}(defn overlay [x] (show x))\n");
    let source = concat!(
        "(ns demo.caller\n",
        "  (:require #?(:clj [demo.old :as old] :cljs [demo.old :as old])))\n",
        "(defn sample [x] [\"🦉\" (demo.old/show x) (old/show x) :demo.old/key])\n",
    );
    fs::write(&old, provider).unwrap();
    fs::write(&caller, source).unwrap();
    let old_uri = uri(&old);
    let new_uri = uri(&new);
    let caller_uri = uri(&caller);
    let mut child = lsp_server();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let reply = |output: &mut BufReader<_>, id| loop {
        let item = read_message(output).unwrap();
        if item["id"] == id {
            break item;
        }
    };
    let open = |file: &str, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":file,"languageId":"clojure","version":1,"text":text
        }}})
    };
    let change = |file: &str, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":2},"contentChanges":[{"text":text}]
        }})
    };
    let move_file = |id, destination: &str| {
        json!({"jsonrpc":"2.0","id":id,"method":"workspace/willRenameFiles","params":{
            "files":[{"oldUri":old_uri,"newUri":destination}]
        }})
    };
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&fixture.0),
            "capabilities":{"workspace":{"workspaceEdit":{"documentChanges":true},"fileOperations":{"willRename":true,"didRename":true}}}
        }}),
    );
    let initialization = reply(&mut output, 1);
    let capabilities = &initialization["result"]["capabilities"]["workspace"]["fileOperations"];
    assert!(capabilities["willRename"]["filters"].is_array());
    assert!(capabilities["didRename"]["filters"].is_array());
    send_message(&mut input, &open(&old_uri, &overlay));
    send_message(&mut input, &open(&caller_uri, source));
    send_message(&mut input, &move_file(2, &uri(&occupied)));
    assert_eq!(reply(&mut output, 2)["error"]["code"], -32803);
    send_message(&mut input, &move_file(3, &new_uri));
    let planned = reply(&mut output, 3);
    let column = |text: &str, line: usize, marker: &str, occurrence: usize| {
        let row = text.lines().nth(line).unwrap();
        let (offset, _) = row.match_indices(marker).nth(occurrence).unwrap();
        row[..offset].encode_utf16().count()
    };
    let edit = |line, start| {
        json!({"range":{"start":{"line":line,"character":start},
                        "end":{"line":line,"character":start+8}},"newText":"demo.new-name"})
    };
    assert_eq!(
        planned["result"],
        json!({"documentChanges":[
            {"textDocument":{"uri":caller_uri,"version":1},"edits":[
                edit(1,column(source,1,"demo.old",0)),
                edit(1,column(source,1,"demo.old",1)),
                edit(2,column(source,2,"demo.old/",0))
            ]},
            {"textDocument":{"uri":old_uri,"version":1},"edits":[edit(0,4)]}
        ]}),
        "{planned}"
    );
    assert!(!new.exists());
    assert_eq!(fs::read_to_string(&old).unwrap(), provider);
    assert_eq!(fs::read_to_string(&caller).unwrap(), source);

    // Simulate the client's text edits and rename on the temporary fixture.
    let edited = source
        .replace("[demo.old :as", "[demo.new-name :as")
        .replace("(demo.old/show", "(demo.new-name/show");
    fs::write(&old, provider.replace("demo.old", "demo.new-name")).unwrap();
    fs::write(&caller, &edited).unwrap();
    send_message(
        &mut input,
        &change(&old_uri, &overlay.replace("demo.old", "demo.new-name")),
    );
    send_message(&mut input, &change(&caller_uri, &edited));
    fs::rename(&old, &new).unwrap();
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","method":"workspace/didRenameFiles","params":{
            "files":[{"oldUri":old_uri,"newUri":new_uri}]
        }}),
    );
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","id":4,"method":"textDocument/definition","params":{
            "textDocument":{"uri":caller_uri},"position":{"line":2,"character":column(&edited,2,"demo.new-name/show",0)}
        }}),
    );
    assert_eq!(reply(&mut output, 4)["result"]["uri"], new_uri);
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","id":5,"method":"textDocument/definition","params":{
            "textDocument":{"uri":new_uri},"position":{"line":2,"character":column(&overlay,2,"show",0)}
        }}),
    );
    assert_eq!(reply(&mut output, 5)["result"]["uri"], new_uri);
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","id":6,"method":"shutdown"}),
    );
    reply(&mut output, 6);
    send_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
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

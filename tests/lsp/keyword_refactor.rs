use super::*;

const COMMAND: &str = "cycle-keyword-auto-resolve";
const TO_NAMESPACED: &str = "Change auto-resolved keyword to namespaced";
const TO_AUTO: &str = "Change namespaced keyword to auto-resolved";
const SOURCE: &str = concat!(
    "(ns demo.keyword)\n",
    "[\"🧪\" ::unicode] ; inline ::ignored\n",
    ":demo.keyword/explicit ; another comment\n",
    "[::same :demo.keyword/same ::same]\n",
);
const OVERLAY: &str = concat!(
    "(ns demo.keyword)\n",
    "[\"🧪\" 0 ::changed] ; inline ::ignored\n",
    ":demo.keyword/explicit ; another comment\n",
    "[::same :demo.keyword/same ::same]\n",
);

fn request_action(id: usize, file: &str, (line, character): (usize, usize), kind: &str) -> Value {
    let position = json!({"line":line,"character":character});
    json!({"jsonrpc":"2.0","id":id,"method":"textDocument/codeAction","params":{
        "textDocument":{"uri":file},"range":{"start":position,"end":position},
        "context":{"diagnostics":[],"only":[kind]}
    }})
}

fn request_command(id: usize, arguments: &Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"workspace/executeCommand","params":{
        "command":COMMAND,"arguments":arguments
    }})
}

fn action(reply: &Value) -> Option<&Value> {
    reply["result"]
        .as_array()
        .unwrap_or_else(|| panic!("expected code actions: {reply}"))
        .iter()
        .find(|action| action["command"]["command"] == COMMAND)
}

fn expected_action(file: &str, (line, character): (usize, usize), title: &str) -> Value {
    json!({"title":title,"kind":"refactor.rewrite","command":{
        "title":title,"command":COMMAND,"arguments":[file,line,character]
    }})
}

fn token_edit((line, start, end): (usize, usize, usize), text: &str) -> Value {
    json!({
        "range":{"start":{"line":line,"character":start},"end":{"line":line,"character":end}},
        "newText":text
    })
}

fn workspace_edit(file: &str, version: usize, edit: &Value) -> Value {
    json!({"documentChanges":[{
        "textDocument":{"uri":file,"version":version},"edits":[edit]
    }]})
}

#[test]
fn keyword_refactor_preserves_utf16_comments_and_the_live_buffer_version() {
    let fixture = Fixture::new("cycle keyword");
    verified_transcript(lsp_server(), &fixture.0, false);
}

#[test]
#[ignore = "requires a Clojure LSP executable"]
fn keyword_refactor_matches_the_selected_edit_from_pinned_reference() {
    let fixture = Fixture::new("cycle keyword parity");
    let binary = std::env::var_os("PARENOX_REFERENCE_LSP").unwrap_or_else(|| "clojure-lsp".into());
    let reference = Command::new(binary)
        .arg("listen")
        .current_dir(&fixture.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let binary = std::env::var_os("PARENOX_LSP_CANDIDATE")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_parenox").into());
    assert_eq!(
        verified_transcript(reference, &fixture.0, true),
        verified_transcript(lsp_server_with(Path::new(&binary)), &fixture.0, false)
    );
}

#[test]
fn keyword_refactor_uses_the_metadata_namespace_and_skips_inactive_tokens() {
    let fixture = Fixture::new("cycle keyword guards");
    let path = fixture.0.join("src/demo.clj");
    let file = uri(&path);
    let source = concat!(
        "(ns ^{:doc \"🧪\"} demo.metadata (:require [clojure.string :as f]))\n",
        "[::same :demo.metadata/same ::same #_::same] ; ::same\n",
        "[:plain :f/foreign ::f/aliased]\n",
        "(in-ns 'demo.other)\n",
        "[::switched :demo.metadata/inactive]\n",
    );
    fs::write(&path, source).unwrap();
    let slash_path = fixture.0.join("src/slash.clj");
    fs::write(&slash_path, "(ns demo.keyword)\n:demo.keyword//\n").unwrap();
    let slash = uri(&slash_path);
    let conditional_path = fixture.0.join("src/conditional.cljc");
    fs::write(
        &conditional_path,
        "#?(:clj (ns demo.keyword) :cljs (ns demo.other))\n::conditional\n",
    )
    .unwrap();
    let conditional = uri(&conditional_path);
    let renamed_core_path = fixture.0.join("src/renamed_core.clj");
    fs::write(
        &renamed_core_path,
        "(ns demo.renamed (:refer-clojure :rename {in-ns jump}))\n(jump 'user)\n::shift\n",
    )
    .unwrap();
    let renamed_core = uri(&renamed_core_path);
    let renamed_require_path = fixture.0.join("src/renamed_require.clj");
    fs::write(
        &renamed_require_path,
        "(ns demo.renamed-require (:require [clojure.core :refer [in-ns] :rename {in-ns jump}]))\n(jump 'user)\n::shift\n",
    )
    .unwrap();
    let renamed_require = uri(&renamed_require_path);
    let dependency = fixture.0.join("vendor");
    fs::create_dir(&dependency).unwrap();
    fs::write(
        fixture.0.join("parenox.edn"),
        "{:paths [\"src\"] :dependency-sources [\"vendor\"]}",
    )
    .unwrap();
    let read_only_path = dependency.join("read_only.clj");
    fs::write(&read_only_path, "(ns read-only)\n::reserved\n").unwrap();
    let read_only = uri(&read_only_path);
    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
    ];
    let queries = [
        (&file, 1, 5),
        (&file, 1, 14),
        (&file, 1, 40),
        (&file, 1, 50),
        (&file, 2, 4),
        (&file, 2, 11),
        (&file, 2, 23),
        (&file, 4, 4),
        (&file, 4, 21),
        (&slash, 1, 6),
        (&conditional, 1, 4),
        (&renamed_core, 2, 4),
        (&renamed_require, 2, 4),
        (&read_only, 1, 3),
    ];
    let filtered_id = 2 + queries.len() * 2;
    for (index, (file, line, character)) in queries.into_iter().enumerate() {
        let id = 2 + index * 2;
        requests.extend([
            request_action(id, file, (line, character), "refactor.rewrite"),
            request_command(id + 1, &json!([file, line, character])),
        ]);
    }
    requests.extend([
        request_action(filtered_id, &file, (1, 5), "refactor.extract"),
        json!({"jsonrpc":"2.0","id":filtered_id+1,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let replies = transcript(&requests);
    let response = |id| replies.iter().find(|message| message["id"] == id).unwrap();
    for (id, cursor, title, expected) in [
        (
            2,
            (1, 5),
            TO_NAMESPACED,
            token_edit((1, 1, 7), ":demo.metadata/same"),
        ),
        (4, (1, 14), TO_AUTO, token_edit((1, 8, 27), "::same")),
    ] {
        assert_eq!(
            action(response(id)),
            Some(&expected_action(&file, cursor, title))
        );
        assert_eq!(
            response(id + 1)["result"]["changes"][&file],
            json!([expected])
        );
    }
    for id in (6..filtered_id).step_by(2) {
        assert!(action(response(id)).is_none(), "{}", response(id));
        assert_eq!(
            response(id + 1)["error"],
            json!({"code":-32600,"message":"Nothing to change."})
        );
    }
    assert!(action(response(filtered_id)).is_none());
    assert!(
        replies
            .iter()
            .all(|message| message["method"] != "window/showMessageRequest")
    );
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

fn verified_transcript(mut child: Child, root: &Path, reference: bool) -> Vec<(Value, Value)> {
    let root = root.canonicalize().unwrap();
    let path = root.join("src/demo_keyword.clj");
    let file = uri(&path);
    fs::write(&path, SOURCE).unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut exchange = |request: Value, applied: Option<bool>| {
        send_message(&mut input, &request);
        if request["id"].is_null() {
            return (Value::Null, Value::Null);
        }
        let mut edit = Value::Null;
        loop {
            let reply = read_message(&mut output).expect("LSP message");
            if reply["id"] == request["id"] && reply["method"].is_null() {
                return (reply, edit);
            }
            if !reply["id"].is_null() && !reply["method"].is_null() {
                let answer = match reply["method"].as_str() {
                    Some("workspace/applyEdit") => {
                        assert!(edit.is_null(), "duplicate workspace/applyEdit: {reply}");
                        edit = reply["params"]["edit"].clone();
                        if applied.expect("unexpected workspace edit") {
                            json!({"applied":true})
                        } else {
                            json!({"applied":false,"failureReason":"version conflict"})
                        }
                    }
                    Some("window/showMessageRequest") => {
                        assert!(
                            reference,
                            "selected-token refactoring must not prompt: {reply}"
                        );
                        assert_eq!(
                            reply["params"]["actions"],
                            json!([{"title":"Yes"},{"title":"No"}])
                        );
                        json!({"title":"No"})
                    }
                    _ => Value::Null,
                };
                send_message(
                    &mut input,
                    &json!({"jsonrpc":"2.0","id":reply["id"],"result":answer}),
                );
            }
        }
    };
    let (initialize, _) = exchange(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&root),"capabilities":{"workspace":{"applyEdit":true,"workspaceEdit":{"documentChanges":true}}}
        }}),
        None,
    );
    let commands = initialize["result"]["capabilities"]["executeCommandProvider"]["commands"]
        .as_array()
        .unwrap();
    assert!(commands.contains(&json!(COMMAND)));
    exchange(
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        None,
    );
    exchange(
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":file,"version":0,"languageId":"clojure","text":SOURCE
        }}}),
        None,
    );

    let mut matched = Vec::new();
    for (id, cursor, title, edit) in [
        (
            2,
            (1, 15),
            TO_NAMESPACED,
            token_edit((1, 6, 15), ":demo.keyword/unicode"),
        ),
        (4, (2, 5), TO_AUTO, token_edit((2, 0, 22), "::explicit")),
        (
            6,
            (3, 5),
            TO_NAMESPACED,
            token_edit((3, 1, 7), ":demo.keyword/same"),
        ),
    ] {
        let (reply, _) = exchange(request_action(id, &file, cursor, "refactor"), None);
        let action = action(&reply).unwrap();
        assert_eq!(action, &expected_action(&file, cursor, title));
        let (reply, applied) = exchange(
            request_command(id + 1, &action["command"]["arguments"]),
            Some(true),
        );
        assert_eq!(applied, workspace_edit(&file, 0, &edit), "{reply}");
        assert_eq!(reply["result"], applied);
        matched.push((action.clone(), applied));
    }

    exchange(
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":7},"contentChanges":[{"text":OVERLAY}]
        }}),
        None,
    );
    let cursor = (1, 14);
    let (reply, _) = exchange(request_action(8, &file, cursor, "refactor.rewrite"), None);
    let action = action(&reply).unwrap();
    assert_eq!(action, &expected_action(&file, cursor, TO_NAMESPACED));
    let expected = workspace_edit(&file, 7, &token_edit((1, 8, 17), ":demo.keyword/changed"));
    for (id, accepted) in [(9, false), (10, true)] {
        let (reply, applied) = exchange(
            request_command(id, &action["command"]["arguments"]),
            Some(accepted),
        );
        assert_eq!(applied, expected, "{reply}");
        if accepted || reference {
            assert_eq!(reply["result"], applied);
        } else {
            assert_eq!(
                reply["error"],
                json!({"code":-32803,"message":"version conflict"})
            );
        }
        matched.push((action.clone(), applied));
    }
    exchange(json!({"jsonrpc":"2.0","id":11,"method":"shutdown"}), None);
    exchange(json!({"jsonrpc":"2.0","method":"exit"}), None);
    drop(input);
    assert!(child.wait().unwrap().success());
    assert_eq!(fs::read_to_string(path).unwrap(), SOURCE);
    matched
}

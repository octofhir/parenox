use super::*;
use parenox_analysis::reader::{self, Form, Kind};

const TITLE: &str = "Change nested if to cond";
const COMMAND: &str = "if->cond-refactor";
const SOURCE: &str = "(ns demo)\n(defn if-nested [x y] [\"🧪\" (if x :a (if y :b :z))])\n";
const OVERLAY: &str =
    "(ns demo)\n(defn if-nested [x y] [\"🧪\" (if (not x) :changed (if y :b))])\n";

fn action_request(id: usize, uri: &str, (line, column): (usize, usize), kind: &str) -> Value {
    let position = json!({"line":line,"character":column});
    json!({"jsonrpc":"2.0","id":id,"method":"textDocument/codeAction","params":{
        "textDocument":{"uri":uri},"range":{"start":position,"end":position},
        "context":{"diagnostics":[],"only":[kind]}
    }})
}

fn execute(id: usize, command: &str, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"workspace/executeCommand","params":{
        "command":command,"arguments":arguments
    }})
}

fn if_action(reply: &Value) -> Option<&Value> {
    reply["result"]
        .as_array()
        .unwrap_or_else(|| panic!("expected code actions: {reply}"))
        .iter()
        .find(|item| item["title"] == TITLE)
}

fn document_edit(uri: &str, version: usize, end: usize, new_text: &str) -> Value {
    json!({"documentChanges":[{
        "textDocument":{"uri":uri,"version":version},"edits":[{
            "range":{"start":{"line":1,"character":28},"end":{"line":1,"character":end}},
            "newText":new_text
        }]
    }]})
}

#[test]
fn if_to_cond_rewrites_nested_else_with_utf16_ranges_and_live_version() {
    let fixture = Fixture::new("if to cond");
    replay(lsp_server(), &fixture.0, false);
}

#[test]
#[ignore = "requires a Clojure LSP executable"]
fn if_to_cond_matches_pinned_reference() {
    let fixture = Fixture::new("if to cond parity");
    let binary = std::env::var_os("PARENOX_REFERENCE_LSP").unwrap_or_else(|| "clojure-lsp".into());
    let reference = Command::new(binary)
        .arg("listen")
        .current_dir(&fixture.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let candidate = std::env::var_os("PARENOX_LSP_CANDIDATE")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_parenox").into());
    assert_eq!(
        replay(reference, &fixture.0, true),
        replay(lsp_server_with(Path::new(&candidate)), &fixture.0, false)
    );
}

#[test]
fn if_to_cond_retains_trivia_and_respects_quoted_incomplete_or_shadowed_forms() {
    let fixture = Fixture::new("if to cond guards");
    let path = fixture.0.join("src/demo.cljc");
    let file = uri(&path);
    let source = concat!(
        "(ns demo)\n",
        "(#_:head if ; before test\n",
        "  x #_:discarded ; before then\n",
        "  :yes ; before else\n",
        "  (if y :no ; final else\n",
        "    nil)\n",
        "  ; trailing\n",
        "  )\n",
        "(defn lexical [cond x] (if x :yes :no))\n",
        "(defn cond [& xs] xs)\n",
        "(if x :yes nil)\n",
        "'(if x :yes :no)\n",
        "#_(if x :yes :no)\n",
        "(if x :yes\n",
    );
    fs::write(&path, source).unwrap();
    let cases = [(1, 0), (8, 23), (10, 0), (11, 1), (12, 2), (13, 0)];
    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
    ];
    for (index, (line, column)) in cases.into_iter().enumerate() {
        let id = 2 + index * 2;
        requests.extend([
            action_request(id, &file, (line, column), "refactor.extract"),
            execute(id + 1, COMMAND, json!([file, line, column])),
        ]);
    }
    requests.extend([
        action_request(14, &file, (1, 0), "refactor.rewrite"),
        json!({"jsonrpc":"2.0","id":15,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let replies = transcript(&requests);
    let response = |id| replies.iter().find(|message| message["id"] == id).unwrap();
    let rewritten = |id| {
        response(id)["result"]["changes"][&file][0]["newText"]
            .as_str()
            .unwrap_or_else(|| panic!("expected a rewrite: {}", response(id)))
    };
    let text = rewritten(3);
    for trivia in [
        "; before test",
        "#_:head",
        "#_:discarded",
        "; before then",
        "; before else",
        "; final else",
        "; trailing",
    ] {
        assert_eq!(text.matches(trivia).count(), 1, "{text}");
    }
    let forms = reader::read(text).unwrap();
    let [
        Form {
            kind: Kind::List(items),
            ..
        },
    ] = forms.as_slice()
    else {
        panic!("expected one valid cond form: {text}");
    };
    let tokens: Vec<_> = items
        .iter()
        .map(|form| &text[form.span.start.offset..form.span.end.offset])
        .collect();
    assert_eq!(tokens, ["cond", "x", ":yes", "y", ":no", ":else", "nil"]);
    for id in [5, 7] {
        assert!(rewritten(id).starts_with("(clojure.core/cond\n"));
    }
    for id in [2, 4, 6] {
        assert!(if_action(response(id)).is_some());
    }
    for id in [8, 10, 12, 14] {
        assert!(if_action(response(id)).is_none(), "{}", response(id));
    }
    for id in [9, 11, 13] {
        assert!(response(id)["result"].is_null());
        assert_eq!(
            response(id)["error"],
            json!({"code":-32602,"message":"Not an if expression"})
        );
    }
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

fn replay(mut child: Child, root: &Path, reference: bool) -> [Value; 4] {
    let root = root.canonicalize().unwrap();
    let path = root.join("src/demo.cljc");
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
                let result = if reply["method"] == "workspace/applyEdit" {
                    assert!(edit.is_null(), "duplicate workspace/applyEdit: {reply}");
                    edit = reply["params"]["edit"].clone();
                    match applied.expect("unexpected workspace edit") {
                        true => json!({"applied":true}),
                        false => json!({"applied":false,"failureReason":"version conflict"}),
                    }
                } else {
                    Value::Null
                };
                send_message(
                    &mut input,
                    &json!({"jsonrpc":"2.0","id":reply["id"],"result":result}),
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
            "uri":file,"version":1,"languageId":"clojure","text":SOURCE
        }}}),
        None,
    );
    let cursor = (1, 28);
    let expected_action = json!({"title":TITLE,"kind":"refactor.extract","command":{
        "title":TITLE,"command":COMMAND,"arguments":[file,cursor.0,cursor.1]
    }});
    let (actions, _) = exchange(action_request(2, &file, cursor, "refactor"), None);
    let saved_action = if_action(&actions)
        .expect("saved conditional code action")
        .clone();
    assert_eq!(saved_action, expected_action);
    let from_action = |id, action: &Value| {
        execute(
            id,
            action["command"]["command"].as_str().unwrap(),
            action["command"]["arguments"].clone(),
        )
    };
    let (saved, mut saved_edit) = exchange(from_action(3, &saved_action), Some(true));
    let indentation = " ".repeat(30);
    let saved_text = format!(
        "(cond\n{indentation}x\n{indentation}:a\n\n{indentation}y\n{indentation}:b\n\n{indentation}:else\n{indentation}:z)"
    );
    assert_eq!(
        saved_edit,
        document_edit(&file, usize::from(!reference), 50, &saved_text),
        "{saved}"
    );
    assert_eq!(saved["result"], saved_edit);
    saved_edit["documentChanges"][0]["textDocument"]["version"] = json!(1);

    exchange(
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":7},"contentChanges":[{"text":OVERLAY}]
        }}),
        None,
    );
    let (actions, _) = exchange(action_request(4, &file, cursor, "refactor.extract"), None);
    let live_action = if_action(&actions)
        .expect("live conditional code action")
        .clone();
    assert_eq!(live_action, expected_action);
    let live_text = format!(
        "(cond\n{indentation}(not x)\n{indentation}:changed\n\n{indentation}y\n{indentation}:b)"
    );
    let live_edit = document_edit(&file, 7, 59, &live_text);
    for (id, accepted) in [(5, false), (6, true)] {
        let (reply, apply) = exchange(from_action(id, &live_action), Some(accepted));
        assert_eq!(apply, live_edit, "{reply}");
        if accepted || reference {
            assert_eq!(reply["result"], apply);
        } else {
            assert_eq!(
                reply["error"],
                json!({"code":-32803,"message":"version conflict"})
            );
        }
    }
    exchange(json!({"jsonrpc":"2.0","id":7,"method":"shutdown"}), None);
    exchange(json!({"jsonrpc":"2.0","method":"exit"}), None);
    drop(input);
    assert!(child.wait().unwrap().success());
    assert_eq!(fs::read_to_string(path).unwrap(), SOURCE);
    [saved_action, live_action, saved_edit, live_edit]
}

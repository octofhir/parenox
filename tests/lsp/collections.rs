use super::*;

const SOURCE: &str = concat!(
    "(ns demo)\n",
    "(f :x 1 2)\n",
    "{:x 1, :y 2}\n",
    "[:x 1, :y 2]\n",
    "#{:x 1 :y 2}\n",
    "[\"🧪\" [:x ; retained\n",
    "       #_:discard 1, :y 2]]\n",
);

fn command(id: usize, file: &str, (line, column): (usize, usize), target: Option<&str>) -> Value {
    let (name, arguments) = match target {
        None => ("cycle-coll", json!([file, line, column])),
        Some(target) => ("change-coll", json!([file, line, column, target])),
    };
    json!({"jsonrpc":"2.0","id":id,"method":"workspace/executeCommand","params":{
        "command":name,"arguments":arguments
    }})
}

fn actions(id: usize, file: &str, (line, column): (usize, usize), only: Option<&str>) -> Value {
    let position = json!({"line":line,"character":column});
    json!({"jsonrpc":"2.0","id":id,"method":"textDocument/codeAction","params":{
        "textDocument":{"uri":file},"range":{"start":position,"end":position},
        "context":{"diagnostics":[],"only":only.map(|kind| [kind])}
    }})
}

fn collection_actions(response: &Value) -> Vec<Value> {
    response["result"]
        .as_array()
        .unwrap_or_else(|| panic!("missing code actions: {response}"))
        .iter()
        .filter(|action| action["command"]["command"] == "change-coll")
        .cloned()
        .collect()
}

fn expected_actions(file: &str, (line, column): (usize, usize), targets: &[&str]) -> Vec<Value> {
    targets.iter().map(|target| json!({
        "title":format!("Change coll to {target}"),"kind":"refactor","command":{
            "title":"Change coll","command":"change-coll","arguments":[file,line,column,target]
        }
    })).collect()
}

fn document_edit(
    file: &str,
    version: usize,
    (start_line, start_col): (usize, usize),
    (end_line, end_col): (usize, usize),
    replacement: &str,
) -> Value {
    json!({"documentChanges":[{
        "textDocument":{"uri":file,"version":version},"edits":[{
            "range":{"start":{"line":start_line,"character":start_col},"end":{"line":end_line,"character":end_col}},
            "newText":replacement
        }]
    }]})
}

#[test]
fn collection_refactorings_preserve_ranges_trivia_and_live_version() {
    let fixture = Fixture::new("collection refactoring");
    verified_transcript(lsp_server(), &fixture.0, false);
}

#[test]
#[ignore = "requires a Clojure LSP executable"]
fn collection_refactorings_match_pinned_reference() {
    let fixture = Fixture::new("collection refactoring parity");
    let reference =
        std::env::var_os("PARENOX_REFERENCE_LSP").unwrap_or_else(|| "clojure-lsp".into());
    let reference = Command::new(reference)
        .arg("listen")
        .current_dir(&fixture.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let candidate = std::env::var_os("PARENOX_LSP_CANDIDATE")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_parenox").into());
    let candidate = lsp_server_with(Path::new(&candidate));
    assert_eq!(
        verified_transcript(reference, &fixture.0, true),
        verified_transcript(candidate, &fixture.0, false)
    );
}

#[test]
fn collection_refactorings_refuse_invalid_targets_and_filter_code_actions() {
    let fixture = Fixture::new("collection refactoring guards");
    let path = fixture.0.join("src/demo.cljc");
    let file = uri(&path);
    fs::write(
        &path,
        concat!(
            "(ns demo)\n",
            "(f 1 2)\n",
            "(:x 1 :x 2)\n",
            "[:x :x]\n",
            "([:x] 1 [:x] 2)\n",
            "[[:x] [:x]]\n",
            "#:prefix{:x 1}\n",
            "(f #?@(:clj [1] :cljs [2 3]))\n",
            "[1 #?(:clj 2 :cljs 1)]\n",
            "[:a 1 :b 2]\n",
            "([:a 1][:b 2])\n",
        ),
    )
    .unwrap();
    let cases = [
        ((1, 0), "map"),
        ((2, 0), "map"),
        ((3, 0), "set"),
        ((4, 0), "map"),
        ((5, 0), "set"),
        ((6, 0), "vector"),
        ((6, 8), "vector"),
        ((7, 0), "map"),
        ((8, 0), "set"),
    ];
    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
    ];
    for (index, (position, target)) in cases.into_iter().enumerate() {
        let id = 2 + index * 3;
        requests.extend([
            actions(id, &file, position, Some("refactor")),
            command(id + 1, &file, position, Some(target)),
            command(id + 2, &file, position, None),
        ]);
    }
    let next = 2 + cases.len() * 3;
    requests.extend([
        actions(next, &file, (9, 0), Some("refactor")),
        actions(next + 1, &file, (9, 0), Some("refactor.rewrite")),
        actions(next + 2, &file, (9, 0), Some("quickfix")),
        actions(next + 3, &file, (9, 0), None),
        command(next + 4, &file, (9, 0), Some("tuple")),
        command(next + 5, &file, (9, 1), None),
        command(next + 6, &file, (9, 10), None),
        command(next + 7, &file, (10, 7), None),
        json!({"jsonrpc":"2.0","id":next + 8,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let replies = transcript(&requests);
    let response = |id| replies.iter().find(|message| message["id"] == id).unwrap();
    for (index, ((line, _), target)) in cases.into_iter().enumerate() {
        let id = index * 3 + 2;
        let menu = collection_actions(response(id));
        assert!(
            menu.iter()
                .all(|action| action["command"]["arguments"][3] != target),
            "{target}: {}",
            response(id)
        );
        if line == 6 {
            assert!(menu.is_empty());
        }
        for request_id in [id + 1, id + 2] {
            assert_eq!(
                response(request_id)["error"]["code"],
                -32600,
                "{}",
                response(request_id)
            );
        }
    }
    let all = expected_actions(&file, (9, 0), &["set", "list", "map"]);
    assert_eq!(collection_actions(response(next)), all);
    assert!(collection_actions(response(next + 1)).is_empty());
    assert!(collection_actions(response(next + 2)).is_empty());
    assert_eq!(collection_actions(response(next + 3)), all);
    assert_eq!(response(next + 4)["error"]["code"], -32602);
    assert_eq!(response(next + 5)["error"]["code"], -32600);
    assert_eq!(response(next + 6)["error"]["code"], -32600);
    let adjacent = &response(next + 7)["result"]["changes"][&file];
    assert_eq!(
        adjacent,
        &json!([{
            "range":{"start":{"line":10,"character":7},"end":{"line":10,"character":13}},
            "newText":"#{:b 2}"
        }])
    );
}

fn verified_transcript(mut child: Child, root: &Path, reference: bool) -> Vec<Value> {
    let root = root.canonicalize().unwrap();
    let path = root.join("src/demo.cljc");
    let file = uri(&path);
    fs::write(&path, SOURCE).unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut exchange = |message: Value, applied: Option<bool>| {
        send_message(&mut input, &message);
        if message["id"].is_null() {
            return (Value::Null, Value::Null);
        }
        let mut edit = Value::Null;
        loop {
            let reply = read_message(&mut output).expect("LSP message");
            if reply["method"].is_null() && reply["id"] == message["id"] {
                return (reply, edit);
            }
            if !reply["method"].is_null() && !reply["id"].is_null() {
                let result = if reply["method"] == "workspace/applyEdit" {
                    assert!(edit.is_null(), "duplicate workspace/applyEdit: {reply}");
                    edit = reply["params"]["edit"].clone();
                    match applied.expect("unexpected workspace/applyEdit") {
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
    let advertised = initialize["result"]["capabilities"]["executeCommandProvider"]["commands"]
        .as_array()
        .unwrap();
    assert!(advertised.contains(&json!("cycle-coll")));
    assert!(advertised.contains(&json!("change-coll")));
    exchange(
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        None,
    );
    exchange(
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":file,"version":1,"languageId":"clojure","text":SOURCE}
        }}),
        None,
    );

    let opened_version = usize::from(!reference);
    let mut captured = Vec::new();
    for (index, (cursor, start, end, replacement, targets)) in [
        (
            (1, 0),
            (1, 0),
            (1, 10),
            "{f :x 1 2}",
            ["vector", "set", "map"],
        ),
        (
            (2, 0),
            (2, 0),
            (2, 12),
            "[:x 1, :y 2]",
            ["vector", "set", "list"],
        ),
        (
            (3, 0),
            (3, 0),
            (3, 12),
            "#{:x 1, :y 2}",
            ["set", "list", "map"],
        ),
        (
            (4, 1),
            (4, 0),
            (4, 12),
            "(:x 1 :y 2)",
            ["vector", "list", "map"],
        ),
        (
            (5, 6),
            (5, 6),
            (6, 26),
            "#{:x ; retained\n       #_:discard 1, :y 2}",
            ["set", "list", "map"],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let id = 2 + index * 2;
        let menu =
            collection_actions(&exchange(actions(id, &file, cursor, Some("refactor")), None).0);
        assert_eq!(menu, expected_actions(&file, cursor, &targets));
        captured.push(json!(menu));
        let (reply, mut edit) = exchange(command(id + 1, &file, cursor, None), Some(true));
        assert_eq!(
            edit,
            document_edit(&file, opened_version, start, end, replacement),
            "{reply}"
        );
        assert_eq!(reply["result"], edit);
        // Clojure LSP starts an opened document at 0 and uses the client version after didChange.
        edit["documentChanges"][0]["textDocument"]["version"] = json!(1);
        captured.push(edit);
    }

    let overlay = SOURCE.replace("#_:discard 1, :y 2", "#_:discard 1, :z 3");
    exchange(
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":7},"contentChanges":[{"text":overlay}]
        }}),
        None,
    );
    let menu = collection_actions(&exchange(actions(12, &file, (5, 6), Some("refactor")), None).0);
    assert_eq!(
        menu,
        expected_actions(&file, (5, 6), &["set", "list", "map"])
    );
    let clicked = &menu[2]["command"];
    let expected = document_edit(
        &file,
        7,
        (5, 6),
        (6, 26),
        "{:x ; retained\n       #_:discard 1, :z 3}",
    );
    for (id, accepted) in [(13, false), (14, true)] {
        let (reply, edit) = exchange(
            json!({"jsonrpc":"2.0","id":id,"method":"workspace/executeCommand","params":{
                "command":clicked["command"],"arguments":clicked["arguments"]
            }}),
            Some(accepted),
        );
        assert_eq!(edit, expected, "{reply}");
        if accepted || reference {
            assert_eq!(reply["result"], edit);
        } else {
            assert_eq!(
                reply["error"],
                json!({"code":-32803,"message":"version conflict"})
            );
        }
        captured.push(edit);
    }
    captured.push(json!(menu));
    exchange(json!({"jsonrpc":"2.0","id":15,"method":"shutdown"}), None);
    exchange(json!({"jsonrpc":"2.0","method":"exit"}), None);
    drop(input);
    assert!(child.wait().unwrap().success());
    assert_eq!(fs::read_to_string(path).unwrap(), SOURCE);
    captured
}

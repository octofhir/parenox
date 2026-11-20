use super::*;
use parenox_analysis::reader::{self, Form, Kind};

const COND_COMMAND: &str = "cond->if-refactor";
const COND_TITLE: &str = "Change cond to nested if";
const IF_COMMAND: &str = "if->cond-refactor";
const IF_TITLE: &str = "Change nested if to cond";
const SOURCE: &str = concat!(
    "(ns demo)\n",
    "(defn cond-nested [x y] [\"🧪\" (cond x :a y :b :else :z)])\n",
    "(defn if-nested [x y] [\"🧪\" (if x :a (if y :b :z))])\n",
);
const OVERLAY: &str = concat!(
    "(ns demo)\n",
    "(defn cond-nested [x y] [\"🧪\" (cond (not x) :changed y :b)])\n",
    "(defn if-nested [x y] [\"🧪\" (if (not x) :changed (if y :b))])\n",
);

fn action_request(id: usize, file: &str, (line, column): (usize, usize)) -> Value {
    let position = json!({"line":line,"character":column});
    json!({"jsonrpc":"2.0","id":id,"method":"textDocument/codeAction","params":{
        "textDocument":{"uri":file},"range":{"start":position,"end":position},
        "context":{"diagnostics":[],"only":["refactor.extract"]}
    }})
}

fn execute(id: usize, command: &str, arguments: &Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"workspace/executeCommand","params":{
        "command":command,"arguments":arguments
    }})
}

fn action<'a>(reply: &'a Value, title: &str) -> Option<&'a Value> {
    reply["result"]
        .as_array()
        .unwrap_or_else(|| panic!("expected code actions: {reply}"))
        .iter()
        .find(|item| item["title"] == title)
}

fn edit(
    file: &str,
    version: usize,
    (line, start, end): (usize, usize, usize),
    new_text: &str,
) -> Value {
    json!({"documentChanges":[{
        "textDocument":{"uri":file,"version":version},"edits":[{
            "range":{"start":{"line":line,"character":start},"end":{"line":line,"character":end}},
            "newText":new_text
        }]
    }]})
}

#[test]
fn cond_to_if_preserves_utf16_and_matches_out_of_order_apply_replies() {
    let fixture = Fixture::new("cond to if");
    replay(lsp_server(), &fixture.0, false);
}

#[test]
#[ignore = "requires a Clojure LSP executable"]
fn cond_to_if_matches_pinned_reference() {
    let fixture = Fixture::new("cond to if parity");
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
        replay(reference, &fixture.0, true),
        replay(lsp_server_with(Path::new(&binary)), &fixture.0, false)
    );
}

#[test]
fn cond_to_if_preserves_trivia_and_requires_an_active_core_cond() {
    let fixture = Fixture::new("cond to if guards");
    let path = fixture.0.join("src/demo.cljc");
    let file = uri(&path);
    let source = concat!(
        "(ns demo)\n",
        "(cond x :a)\n",
        "(cond x :a :else :z)\n",
        "(cond :truthy :z)\n",
        "(cond)\n",
        "(clojure.core/cond x :a :else :z)\n",
        "(#_:head cond ; first test\n",
        "  x #_:discarded ; first result\n",
        "  :a ; second test\n",
        "  y ; second result\n",
        "  :b ; else test\n",
        "  :else #_:default ; else result\n",
        "  :z ; trailing\n",
        "  )\n",
        "(cond x :a :else)\n",
        "'(cond x :a)\n",
        "(defn shadow [cond x] (cond x :a))\n",
        "(vector(cond)42)\n",
        "(cond ::flag :a)\n",
    );
    fs::write(&path, source).unwrap();
    let positions = [
        (1, 0),
        (2, 0),
        (3, 0),
        (4, 0),
        (5, 0),
        (6, 0),
        (14, 0),
        (15, 1),
        (16, 22),
        (17, 7),
        (18, 0),
    ];
    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
    ];
    for (index, (line, column)) in positions.into_iter().enumerate() {
        let id = 2 + index * 2;
        requests.extend([
            action_request(id, &file, (line, column)),
            execute(id + 1, COND_COMMAND, &json!([file, line, column])),
        ]);
    }
    requests.extend([
        json!({"jsonrpc":"2.0","id":24,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let replies = transcript(&requests);
    let response = |id| replies.iter().find(|message| message["id"] == id).unwrap();
    let rewrite = |id| {
        response(id)["result"]["changes"][&file][0]["newText"]
            .as_str()
            .unwrap_or_else(|| panic!("missing edit: {}", response(id)))
    };
    for (id, expected) in [
        (3, "(if x\n  :a)"),
        (5, "(if x\n  :a\n  :z)"),
        (7, "(if true\n  :z)"),
        (9, "nil"),
        (11, "(if x\n  :a\n  :z)"),
        (21, " nil "),
        (23, "(if ::flag\n  :a)"),
    ] {
        assert_eq!(rewrite(id), expected);
    }
    let text = rewrite(13);
    for marker in [
        "#_:head",
        "; first test",
        "#_:discarded",
        "; first result",
        "; second test",
        "; second result",
        "; else test",
        "#_:default",
        "; else result",
        "; trailing",
    ] {
        assert_eq!(text.matches(marker).count(), 1, "{text}");
    }
    let forms = reader::read(text).unwrap();
    let [
        Form {
            kind: Kind::List(outer),
            ..
        },
    ] = forms.as_slice()
    else {
        panic!("expected one outer if: {text}");
    };
    let [
        if_head,
        test,
        success,
        Form {
            kind: Kind::List(inner),
            ..
        },
    ] = outer.as_slice()
    else {
        panic!("expected a nested if: {text}");
    };
    assert_eq!(if_head.symbol(), Some("if"));
    assert_eq!(test.symbol(), Some("x"));
    assert_eq!(success.keyword(), Some(":a"));
    let [if_head, test, success, otherwise] = inner.as_slice() else {
        panic!("expected the second case and default: {text}");
    };
    assert_eq!(if_head.symbol(), Some("if"));
    assert_eq!(test.symbol(), Some("y"));
    assert_eq!(success.keyword(), Some(":b"));
    assert_eq!(otherwise.keyword(), Some(":z"));
    for id in [2, 4, 6, 8, 10, 12, 20, 22] {
        assert!(action(response(id), COND_TITLE).is_some());
    }
    for id in [14, 16, 18] {
        assert!(action(response(id), COND_TITLE).is_none());
        assert!(response(id + 1)["result"].is_null());
        assert_eq!(response(id + 1)["error"]["code"], -32602);
    }
    for id in [17, 19] {
        assert_eq!(response(id)["error"]["message"], "Not a cond");
    }
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

fn replay(mut child: Child, root: &Path, reference: bool) -> [Value; 6] {
    let root = root.canonicalize().unwrap();
    let path = root.join("src/demo.cljc");
    let file = uri(&path);
    fs::write(&path, SOURCE).unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let initialize = round_trip(
        &mut input,
        &mut output,
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&root),"capabilities":{"workspace":{"applyEdit":true,"workspaceEdit":{"documentChanges":true}}}
        }}),
    );
    let commands = initialize["result"]["capabilities"]["executeCommandProvider"]["commands"]
        .as_array()
        .unwrap();
    assert!(commands.contains(&json!(COND_COMMAND)) && commands.contains(&json!(IF_COMMAND)));
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    );
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":file,"version":0,"languageId":"clojure","text":SOURCE
        }}}),
    );
    let expected_action = |(line, column), title, command| {
        json!({"title":title,"kind":"refactor.extract","command":{
            "title":title,"command":command,"arguments":[file,line,column]
        }})
    };
    let (cond_pos, if_pos) = ((1, 30), (2, 28));
    let requests = |id, action: &Value| {
        execute(
            id,
            action["command"]["command"].as_str().unwrap(),
            &action["command"]["arguments"],
        )
    };
    let saved_actions = round_trip(&mut input, &mut output, &action_request(2, &file, cond_pos));
    let saved_action = action(&saved_actions, COND_TITLE).unwrap().clone();
    assert_eq!(
        saved_action,
        expected_action(cond_pos, COND_TITLE, COND_COMMAND)
    );
    let saved_apply = round_trip(&mut input, &mut output, &requests(3, &saved_action));
    let indent = " ".repeat(32);
    let outer_saved_text =
        format!("(if x\n{indent}:a\n{indent}(if y\n{indent}  :b\n{indent}  :z))");
    let saved_edit = edit(&file, 0, (1, 30, 55), &outer_saved_text);
    assert_eq!(
        saved_apply["method"], "workspace/applyEdit",
        "{saved_apply}"
    );
    assert_eq!(saved_apply["params"]["edit"], saved_edit);
    let saved_reply = round_trip(
        &mut input,
        &mut output,
        &json!({"jsonrpc":"2.0","id":saved_apply["id"],"result":{"applied":true}}),
    );
    assert_eq!(saved_reply["id"], 3);
    assert_eq!(saved_reply["result"], saved_edit);

    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":7},"contentChanges":[{"text":OVERLAY}]
        }}),
    );
    let newer_cond_actions =
        round_trip(&mut input, &mut output, &action_request(4, &file, cond_pos));
    let live_cond_action = action(&newer_cond_actions, COND_TITLE).unwrap().clone();
    assert_eq!(live_cond_action, saved_action);
    let newer_if_actions = round_trip(&mut input, &mut output, &action_request(5, &file, if_pos));
    let live_if_action = action(&newer_if_actions, IF_TITLE).unwrap().clone();
    assert_eq!(
        live_if_action,
        expected_action(if_pos, IF_TITLE, IF_COMMAND)
    );
    let older = round_trip(&mut input, &mut output, &requests(6, &live_cond_action));
    let old_text = format!("(if (not x)\n{indent}:changed\n{indent}(if y\n{indent}  :b))");
    let old_edit = edit(&file, 7, (1, 30, 58), &old_text);
    assert_eq!(older["method"], "workspace/applyEdit", "{older}");
    assert_eq!(older["params"]["edit"], old_edit);
    let newer = round_trip(&mut input, &mut output, &requests(7, &live_if_action));
    let indent = " ".repeat(30);
    let new_text = format!("(cond\n{indent}(not x)\n{indent}:changed\n\n{indent}y\n{indent}:b)");
    let new_edit = edit(&file, 7, (2, 28, 59), &new_text);
    assert_eq!(newer["method"], "workspace/applyEdit", "{newer}");
    assert_ne!(newer["id"], older["id"]);
    assert_eq!(newer["params"]["edit"], new_edit);
    let accepted = round_trip(
        &mut input,
        &mut output,
        &json!({"jsonrpc":"2.0","id":newer["id"],"result":{"applied":true}}),
    );
    assert_eq!(accepted["id"], 7);
    assert_eq!(accepted["result"], new_edit);
    let rejected = round_trip(
        &mut input,
        &mut output,
        &json!({"jsonrpc":"2.0","id":older["id"],"result":{"applied":false,"failureReason":"version conflict"}}),
    );
    assert_eq!(rejected["id"], 6);
    if reference {
        assert_eq!(rejected["result"], old_edit);
    } else {
        assert_eq!(
            rejected["error"],
            json!({"code":-32803,"message":"version conflict"})
        );
    }
    let shutdown = round_trip(
        &mut input,
        &mut output,
        &json!({"jsonrpc":"2.0","id":8,"method":"shutdown"}),
    );
    assert_eq!(shutdown["id"], 8);
    send_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
    assert_eq!(fs::read_to_string(path).unwrap(), SOURCE);
    [
        saved_action,
        live_cond_action,
        live_if_action,
        saved_edit,
        old_edit,
        new_edit,
    ]
}

fn round_trip(input: &mut impl Write, output: &mut impl BufRead, request: &Value) -> Value {
    send_message(input, request);
    loop {
        let reply = read_message(output).expect("LSP message");
        if reply["id"].is_null() {
            continue;
        }
        if reply["method"].is_null() || reply["method"] == "workspace/applyEdit" {
            return reply;
        }
        send_message(
            input,
            &json!({"jsonrpc":"2.0","id":reply["id"],"result":null}),
        );
    }
}

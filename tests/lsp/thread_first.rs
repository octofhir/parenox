use super::*;

const SOURCE: &str = include_str!("../fixtures/thread_first.clj");

fn command(id: usize, uri: &str, line: usize, character: usize) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"workspace/executeCommand","params":{
        "command":"thread-first","arguments":[uri,line,character]
    }})
}

#[test]
fn thread_first_rewrites_one_call_and_keeps_its_trivia() {
    let fixture = Fixture::new("thread first");
    let path = fixture.0.join("src/demo.clj");
    let file = uri(&path);
    fs::write(&path, SOURCE).unwrap();
    let expected = [
        (
            1,
            17,
            1,
            48,
            "(-> (into {} xs)\n                     (assoc :done true))",
        ),
        (
            1,
            24,
            1,
            36,
            "(-> {}\n                            (into xs))",
        ),
        (2, 17, 2, 35, "(-> (reverse xs)\n                     vec)"),
        (
            4,
            2,
            6,
            15,
            "(-> (into {} xs)\n      (assoc ; between head and input\n    ; after input\n    :done true))",
        ),
        (
            11,
            23,
            11,
            41,
            "(-> (reverse xs)\n                           vec)",
        ),
        (
            12,
            21,
            12,
            29,
            "(clojure.core/-> xs\n                                      vec)",
        ),
        (13, 0, 13, 34, "(-> (into {} [])\n    (assoc, :done, true))"),
    ];
    let refused = [
        (1, 18),
        (1, 47),
        (7, 22),
        (8, 20),
        (9, 19),
        (10, 24),
        (14, 23),
    ];
    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
    ];
    requests.extend(
        expected
            .iter()
            .enumerate()
            .map(|(index, &(line, col, _, _, _))| command(index + 2, &file, line, col)),
    );
    requests.extend(
        refused
            .iter()
            .enumerate()
            .map(|(index, &(line, col))| command(index + 2 + expected.len(), &file, line, col)),
    );
    requests.extend([
        json!({"jsonrpc":"2.0","id":100,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let responses = transcript(&requests);
    let response = |id: usize| {
        responses
            .iter()
            .find(|message| message["id"] == id)
            .unwrap()
    };
    let commands = response(1)["result"]["capabilities"]["executeCommandProvider"]["commands"]
        .as_array()
        .unwrap();
    assert!(commands.contains(&json!("thread-first")));
    for (index, &(line, col, end_line, end_col, text)) in expected.iter().enumerate() {
        let reply = response(index + 2);
        assert_eq!(
            reply["result"]["changes"][&file],
            json!([{
                "range":{"start":{"line":line,"character":col},"end":{"line":end_line,"character":end_col}},
                "newText":text
            }]),
            "{reply}"
        );
    }
    for index in 0..refused.len() {
        assert_eq!(
            response(index + 2 + expected.len())["error"],
            json!({"code":-32600,"message":"Nothing to change."})
        );
    }
    assert_eq!(fs::read_to_string(path).unwrap(), SOURCE);
}

#[test]
fn thread_first_applies_the_current_unsaved_version() {
    let fixture = Fixture::new("thread first unsaved");
    let path = fixture.0.join("src/demo.clj");
    let file = uri(&path);
    fs::write(&path, SOURCE).unwrap();
    let overlay = SOURCE.replace(
        "(assoc (into {} xs) :done true)",
        "(merge (into {} xs) {:seen true})",
    );
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
    send_message(&mut input, &command(2, &file, 1, 17));
    let request = next();
    assert_eq!(request["method"], "workspace/applyEdit", "{request}");
    let edit = &request["params"]["edit"];
    assert_eq!(
        edit["documentChanges"][0],
        json!({
            "textDocument":{"uri":file,"version":7},"edits":[{
                "range":{"start":{"line":1,"character":17},"end":{"line":1,"character":50}},
                "newText":"(-> (into {} xs)\n                     (merge {:seen true}))"
            }]
        })
    );
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","id":request["id"],"result":{"applied":true}}),
    );
    let applied = next();
    assert_eq!(applied["id"], 2);
    assert_eq!(&applied["result"], edit);
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","id":3,"method":"shutdown"}),
    );
    assert_eq!(next()["id"], 3);
    send_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"}));
    assert!(child.wait().unwrap().success());
    assert_eq!(fs::read_to_string(path).unwrap(), SOURCE);
}

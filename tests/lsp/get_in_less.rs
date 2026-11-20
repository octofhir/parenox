use super::*;
use parenox_analysis::reader::{self, Form, Kind};

const COMMAND: &str = "get-in-less";
const TITLE: &str = "Remove one element from get/get-in";
const SOURCE: &str = concat!(
    "(ns demo.lookups (:require [clojure.core :as c]))\n",
    "(defn saved [m] [\"🧪\" (get-in {:x m} [:x :y :z])])\n",
    "(defn one [m] (get-in {:x m} [:x]))\n",
    "(defn two [m] (get-in {:x m} [:x :y]))\n",
    "(defn keyword-get [m] (get {:x m} :x))\n",
    "(defn quoted-first [m] (get-in m ['x :y 'z]))\n",
    "(defn keyword-get-default [m default!] (get m :x (default!)))\n",
    "(defn quoted-get-default [m default!] (get m 'x (default!)))\n",
    "(defn quoted-operator-get [m] (get m (quote x)))\n",
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

fn expected_action(file: &str, (line, character): (usize, usize)) -> Value {
    json!({"title":TITLE,"kind":"refactor.rewrite","command":{
        "title":TITLE,"command":COMMAND,"arguments":[file,line,character]
    }})
}

fn workspace_edit(
    file: &str,
    version: usize,
    (line, start, end): (usize, usize, usize),
    text: &str,
) -> Value {
    json!({"documentChanges":[{
        "textDocument":{"uri":file,"version":version},"edits":[{
            "range":{"start":{"line":line,"character":start},"end":{"line":line,"character":end}},
            "newText":text
        }]
    }]})
}

#[test]
fn get_in_less_preserves_utf16_ranges_and_live_buffer_version() {
    let fixture = Fixture::new("get in less");
    verified_transcript(lsp_server(), &fixture.0, false);
}

#[test]
#[ignore = "requires a Clojure LSP executable"]
fn get_in_less_matches_pinned_reference_for_proven_lookups() {
    let fixture = Fixture::new("get in less parity");
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
fn get_in_less_preserves_lookup_spelling_trivia_and_the_remaining_path() {
    let fixture = Fixture::new("get in less refinement");
    let path = fixture.0.join("src/guards.clj");
    let file = uri(&path);
    let source = concat!(
        "(ns demo.guards (:require [clojure.core :as c]))\n",
        "(defn aliased [m] (c/get-in m [:x :y :z]))\n",
        "(defn qualified [m] (clojure.core/get-in m [:x :y :z]))\n",
        "(defn qualified-get [m] (clojure.core/get m :x))\n",
        "(defn generated-get [m get] (get-in m [:x :y]))\n",
        "(defn destructured-get [{:keys [:demo/get]} m] (get-in m [:x :y]))\n",
        "(defn dynamic-last [m key!] (get-in m [:x (key!)]))\n",
        "(defn dynamic-suffix [m key!] (get-in m [:x :y (key!)]))\n",
        "(defn trivia [m]\n",
        "  (get-in #_:outer ; head\n",
        "    m ; receiver\n",
        "    [ ; before path\n",
        "      #_:first :x ; after first\n",
        "      :y #_:second ; after second\n",
        "      :z ; final key\n",
        "    ] #_:end ; after path\n",
        "  ))\n",
    );
    fs::write(&path, source).unwrap();
    let expected = [
        (1, "(c/get-in", "(c/get-in ((quote :x) m) [:y :z])"),
        (
            2,
            "(clojure.core/get-in",
            "(clojure.core/get-in ((quote :x) m) [:y :z])",
        ),
        (3, "(clojure.core/get", "((quote :x) m)"),
        (4, "(get-in", "(clojure.core/get ((quote :x) m) :y)"),
        (5, "(get-in", "(clojure.core/get ((quote :x) m) :y)"),
        (6, "(get-in", "(get ((quote :x) m) (key!))"),
        (7, "(get-in", "(get-in ((quote :x) m) [:y (key!)])"),
    ];
    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
    ];
    for (index, (line, needle, _)) in expected.iter().enumerate() {
        let cursor = position(source, *line, needle);
        let id = 2 + index * 2;
        requests.extend([
            request_action(id, &file, cursor, "refactor.rewrite"),
            request_command(id + 1, &json!([file, cursor.0, cursor.1])),
        ]);
    }
    requests.extend([
        request_action(16, &file, (9, 2), "refactor"),
        request_command(17, &json!([file, 9, 2])),
        request_action(18, &file, (9, 2), "refactor.extract"),
        json!({"jsonrpc":"2.0","id":19,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let replies = transcript(&requests);
    let response = |id| replies.iter().find(|message| message["id"] == id).unwrap();
    let new_text = |id| {
        response(id)["result"]["changes"][&file][0]["newText"]
            .as_str()
            .unwrap_or_else(|| panic!("expected a rewrite: {}", response(id)))
    };
    for (index, (line, needle, text)) in expected.into_iter().enumerate() {
        let id = 2 + index * 2;
        assert_eq!(
            action(response(id)),
            Some(&expected_action(&file, position(source, line, needle)))
        );
        assert_eq!(new_text(id + 1), text, "line {line}");
    }
    assert_eq!(action(response(16)), Some(&expected_action(&file, (9, 2))));
    assert!(action(response(18)).is_none());
    let rewritten = new_text(17);
    for marker in [
        "#_:outer",
        "; head",
        "; receiver",
        "; before path",
        "#_:first",
        "; after first",
        "#_:second",
        "; after second",
        "; final key",
        "#_:end",
        "; after path",
    ] {
        assert_eq!(rewritten.matches(marker).count(), 1, "{rewritten}");
    }
    let forms = reader::read(rewritten).unwrap();
    let [
        Form {
            kind: Kind::List(items),
            ..
        },
    ] = forms.as_slice()
    else {
        panic!("expected a single lookup: {rewritten}");
    };
    let [
        head,
        Form {
            kind: Kind::List(receiver),
            ..
        },
        path,
    ] = items.as_slice()
    else {
        panic!("expected get-in with a rewritten receiver and two remaining keys: {rewritten}");
    };
    let [
        Form {
            kind: Kind::List(key),
            ..
        },
        value,
    ] = receiver.as_slice()
    else {
        panic!("expected a keyword receiver: {rewritten}");
    };
    assert_eq!(head.symbol(), Some("get-in"));
    assert_eq!(key[0].symbol(), Some("quote"));
    assert_eq!(key[1].keyword(), Some(":x"));
    assert_eq!(value.symbol(), Some("m"));
    assert_eq!(
        path.vector()
            .unwrap()
            .iter()
            .map(Form::keyword)
            .collect::<Vec<_>>(),
        [Some(":y"), Some(":z")]
    );
}

#[test]
fn get_in_less_refuses_unproved_operators_keys_arity_and_evaluation() {
    let fixture = Fixture::new("get in less refusals");
    let folder = fixture.0.join("src/demo");
    fs::create_dir(&folder).unwrap();
    let file = uri(&folder.join("refusals.clj"));
    let source = concat!(
        "(ns demo.refusals)\n",
        "(defn dynamic-key [m k] (get m k))\n",
        "(defn invalid-vector-key [m] (get m [:x :y]))\n",
        "(defn invalid-numeric-key [m] (get m 0))\n",
        "(defn dynamic-first [m key!] (get-in m [(key!) :x :y]))\n",
        "(defn dynamic-path [m path] (get-in m path))\n",
        "(defn empty-path [m] (get-in m []))\n",
        "(defn quoted-path [m] (get-in m '[:x :y]))\n",
        "(defn one-default [m fallback!] (get-in m [:x] (fallback!)))\n",
        "(defn two-default [m fallback!] (get-in m [:x :y] (fallback!)))\n",
        "(defn three-default [m fallback!] (get-in m [:x :y :z] (fallback!)))\n",
        "(defn quoted [m] '(get-in m [:x :y]))\n",
        "(defn discarded [m] #_(get-in m [:x :y]) nil)\n",
        "(defn shadow [m get-in] (get-in m [:x :y]))\n",
    );
    fs::write(folder.join("refusals.clj"), source).unwrap();
    let mut cases: Vec<_> = [
        (1, "(get"),
        (2, "(get"),
        (3, "(get"),
        (4, "(get-in"),
        (5, "(get-in"),
        (6, "(get-in"),
        (7, "(get-in"),
        (8, "(get-in"),
        (9, "(get-in"),
        (10, "(get-in"),
        (11, "(get-in"),
        (12, "(get-in"),
        (13, "(get-in"),
        (6, " (get-in"),
        (6, "get-in m"),
    ]
    .map(|(line, needle)| (file.clone(), position(source, line, needle)))
    .into();
    fs::write(
        folder.join("fake.clj"),
        "(ns demo.fake)\n(defn get-in [m p] [m p])\n",
    )
    .unwrap();
    for (name, source, calls) in [
        (
            "macro.clj",
            "(ns demo.macro)\n(defmacro as-data [form] (list 'quote form))\n(defn f [m] (as-data (get-in m [:x :y])))\n",
            &[(2, "(get-in")][..],
        ),
        (
            "foreign.clj",
            "(ns demo.foreign (:require [demo.fake :as c]))\n(defn f [m] (c/get-in m [:x :y]))\n",
            &[(1, "(c/get-in")][..],
        ),
        (
            "portable.cljc",
            "(ns demo.portable)\n(defn dynamic-last [m key!] (get-in m [:x (key!)]))\n(defn dynamic-suffix [m key!] (get-in m [:x :y (key!)]))\n(defn conditional [m] #?(:clj :jvm :cljs (get-in m [:x :y])))\n",
            &[(1, "(get-in"), (2, "(get-in"), (3, "(get-in")][..],
        ),
    ] {
        let path = folder.join(name);
        fs::write(&path, source).unwrap();
        let file = uri(&path);
        cases.extend(
            calls
                .iter()
                .map(|(line, needle)| (file.clone(), position(source, *line, needle))),
        );
    }
    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
    ];
    for (index, (file, position)) in cases.iter().enumerate() {
        let id = 2 + index * 2;
        requests.extend([
            request_action(id, file, *position, "refactor.rewrite"),
            request_command(id + 1, &json!([file, position.0, position.1])),
        ]);
    }
    requests.extend([
        json!({"jsonrpc":"2.0","id":2+cases.len()*2,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let replies = transcript(&requests);
    let response = |id| replies.iter().find(|message| message["id"] == id).unwrap();
    for (index, (file, cursor)) in cases.iter().enumerate() {
        let id = 2 + index * 2;
        assert!(
            action(response(id)).is_none(),
            "{file} {cursor:?}: {}",
            response(id)
        );
        assert_eq!(
            response(id + 1)["error"],
            json!({"code":-32600,"message":"Nothing to change."}),
            "{file} {cursor:?}"
        );
    }
}

fn position(source: &str, line: usize, needle: &str) -> (usize, usize) {
    let row = source.lines().nth(line).unwrap();
    (
        line,
        row[..row.find(needle).unwrap()].encode_utf16().count(),
    )
}
fn verified_transcript(mut child: Child, root: &Path, reference: bool) -> Vec<(Value, Value)> {
    let root = root.canonicalize().unwrap();
    let path = root.join("src/demo/lookups.clj");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
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
            "uri":file,"version":0,"languageId":"clojure","text":SOURCE
        }}}),
        None,
    );
    let mut matched = Vec::new();
    let overlay = SOURCE
        .replacen("🧪", "🧪🦀", 1)
        .replacen("[:x :y :z]", "[:new :x :y :z]", 1);
    for (id, range @ (line, start, _), edit) in [
        (2, (1, 22, 48), "(get-in (:x {:x m}) [:y :z])"),
        (4, (2, 14, 34), "(:x {:x m})"),
        (6, (3, 14, 37), "(get (:x {:x m}) :y)"),
        (8, (4, 22, 37), "(:x {:x m})"),
        (10, (5, 23, 44), "(get-in ('x m) [:y 'z])"),
        (12, (6, 39, 60), "(:x m (default!))"),
        (14, (7, 38, 59), "('x m (default!))"),
        (16, (8, 30, 47), "((quote x) m)"),
    ] {
        let (reply, _) = exchange(request_action(id, &file, (line, start), "refactor"), None);
        let action = action(&reply).unwrap();
        assert_eq!(action, &expected_action(&file, (line, start)));
        let (reply, applied) = exchange(
            request_command(id + 1, &action["command"]["arguments"]),
            Some(true),
        );
        assert_eq!(applied, workspace_edit(&file, 0, range, edit), "{reply}");
        assert_eq!(reply["result"], applied);
        matched.push((action.clone(), applied));
    }
    exchange(
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":7},"contentChanges":[{"text":overlay}]
        }}),
        None,
    );
    let cursor = (1, 24);
    let (reply, _) = exchange(request_action(18, &file, cursor, "refactor.rewrite"), None);
    let action = action(&reply).unwrap();
    assert_eq!(action, &expected_action(&file, cursor));
    let expected = workspace_edit(&file, 7, (1, 24, 55), "(get-in (:new {:x m}) [:x :y :z])");
    for (id, accepted) in [(19, false), (20, true)] {
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
    exchange(json!({"jsonrpc":"2.0","id":21,"method":"shutdown"}), None);
    exchange(json!({"jsonrpc":"2.0","method":"exit"}), None);
    drop(input);
    assert!(child.wait().unwrap().success());
    assert_eq!(fs::read_to_string(path).unwrap(), SOURCE);
    matched
}

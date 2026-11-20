use super::*;
use parenox_analysis::reader::{self, Form, Kind};

const COMMAND: &str = "get-in-none";
const TITLE: &str = "Unwind whole get/get-in";
const SOURCE: &str = concat!(
    "(ns demo.lookups)\n",
    "(defn saved [m] [\"🧪\" (get-in m ['x 'y 'z])])\n",
    "(defn one [m] (get-in {:x m} [:x]))\n",
    "(defn mixed [m] (get-in {:x m} [:x 'y]))\n",
    "(defn explicit-quote [m] (get-in m [(quote x) 'y]))\n",
    "(defn keyword-get [m] (get {:x m} :x))\n",
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

fn request_command(id: usize, command: &str, arguments: &Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"workspace/executeCommand","params":{
        "command":command,"arguments":arguments
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

fn position(source: &str, line: usize, needle: &str) -> (usize, usize) {
    let row = source.lines().nth(line).unwrap();
    (
        line,
        row[..row.find(needle).unwrap()].encode_utf16().count(),
    )
}

#[test]
fn get_in_none_preserves_utf16_ranges_and_live_buffer_version() {
    let fixture = Fixture::new("get in none");
    verified_transcript(lsp_server(), &fixture.0, false);
}

#[test]
#[ignore = "requires a Clojure LSP executable"]
fn get_in_none_matches_pinned_reference_when_dispatch_agrees() {
    let fixture = Fixture::new("get in none parity");
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
fn get_in_none_round_trips_through_get_in_all_on_the_edited_buffer() {
    let fixture = Fixture::new("get in none round trip");
    let path = fixture.0.join("src/lookups.clj");
    let file = uri(&path);
    let initialize =
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}});
    let shutdown = json!({"jsonrpc":"2.0","id":3,"method":"shutdown"});
    let exit = json!({"jsonrpc":"2.0","method":"exit"});
    let run = |command, args: &Value, change: Option<Value>| {
        let mut requests = vec![initialize.clone()];
        requests.extend(change);
        requests.extend([
            request_command(2, command, args),
            shutdown.clone(),
            exit.clone(),
        ]);
        let replies = transcript(&requests);
        let response = replies.iter().find(|message| message["id"] == 2).unwrap();
        response["result"]["changes"][&file][0]["newText"]
            .as_str()
            .unwrap_or_else(|| panic!("expected a rewrite: {response}"))
            .to_owned()
    };
    for (lookup, expected, consolidated) in [
        (
            "(get-in m [:x 'y :z])",
            "((quote :z) ('y ((quote :x) m)))",
            "(get-in m [(quote :x) 'y (quote :z)])",
        ),
        (
            "(get-in (:w m) [:x :y])",
            "((quote :y) ((quote :x) (:w m)))",
            "(get-in (:w m) [(quote :x) (quote :y)])",
        ),
        (
            "(get-in {:x m} [:x :y])",
            "((quote :y) (:x {:x m}))",
            "(get-in {:x m} [:x (quote :y)])",
        ),
    ] {
        let source = format!("(ns demo.lookups)\n(defn lookup [m] {lookup})\n");
        fs::write(&path, &source).unwrap();
        let cursor = position(&source, 1, lookup);
        let args = json!([file, cursor.0, cursor.1]);
        let unwound = run(COMMAND, &args, None);
        assert_eq!(unwound, expected);
        let overlay = source.replace(lookup, &unwound);
        let round_trip = run(
            "get-in-all",
            &args,
            Some(json!({
                "jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
                    "uri":file,"version":7,"languageId":"clojure","text":overlay
                }}
            })),
        );
        assert_eq!(round_trip, consolidated);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
}

#[test]
fn get_in_none_keeps_the_replacement_within_the_reader_nesting_limit() {
    let fixture = Fixture::new("get in none nesting");
    let path = fixture.0.join("src/lookup.clj");
    let file = uri(&path);
    let keys = ":x ".repeat(reader::MAX_NESTING_DEPTH + 1);
    let source = format!("(ns demo.lookup)\n(defn lookup [m] (get-in m [{keys}]))\n");
    assert!(reader::read(&source).is_ok());
    fs::write(&path, &source).unwrap();
    let cursor = position(&source, 1, "(get-in");
    let replies = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        request_action(2, &file, cursor, "refactor.rewrite"),
        request_command(3, COMMAND, &json!([file, cursor.0, cursor.1])),
        json!({"jsonrpc":"2.0","id":4,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| replies.iter().find(|message| message["id"] == id).unwrap();
    assert!(action(response(2)).is_none());
    assert!(
        response(2)["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| { action["command"]["command"] == "get-in-less" })
    );
    assert_eq!(response(3)["error"]["code"], -32600);
}

#[test]
fn get_in_none_unwinds_all_keys_preserving_generic_dispatch_and_trivia() {
    let fixture = Fixture::new("get in none refinement");
    let path = fixture.0.join("src/lookups.clj");
    let file = uri(&path);
    let source = concat!(
        "(ns demo.lookups (:require [clojure.core :as c]))\n",
        "(defn aliased [m] (c/get-in m [:x :y :z]))\n",
        "(defn qualified [m] (clojure.core/get-in {:x m} [:x :y :z]))\n",
        "(defn quoted [m] (get-in m [':x (quote y) (quote :z)]))\n",
        "(defn get-default [m f] (c/get m :x (f)))\n",
        "(defn trivia [m]\n",
        "  (get-in #_:outer ; after lookup\n",
        "    m ; after receiver\n",
        "    [ ; before first key\n",
        "      #_:first :x ; after first key\n",
        "      :y #_:second ; after second key\n",
        "      :z ; after last key\n",
        "    ] #_:end ; after path\n",
        "  ))\n",
    );
    fs::write(&path, source).unwrap();
    let expected = [
        (1, "(c/get-in", "((quote :z) ((quote :y) ((quote :x) m)))"),
        (
            2,
            "(clojure.core/get-in",
            "((quote :z) ((quote :y) (:x {:x m})))",
        ),
        (3, "(get-in", "((quote :z) ((quote y) (':x m)))"),
        (4, "(c/get", "(:x m (f))"),
    ];
    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
    ];
    for (index, (line, needle, _)) in expected.iter().enumerate() {
        let cursor = position(source, *line, needle);
        let id = 2 + index * 2;
        requests.extend([
            request_action(id, &file, cursor, "refactor.rewrite"),
            request_command(id + 1, COMMAND, &json!([file, cursor.0, cursor.1])),
        ]);
    }
    let aliased = position(source, 1, "(c/get-in");
    requests.extend([
        request_action(10, &file, (6, 2), "refactor"),
        request_command(11, COMMAND, &json!([file, 6, 2])),
        request_command(12, "get-in-less", &json!([file, aliased.0, aliased.1])),
        request_action(13, &file, (6, 2), "refactor.extract"),
        json!({"jsonrpc":"2.0","id":14,"method":"shutdown"}),
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
    assert_eq!(action(response(10)), Some(&expected_action(&file, (6, 2))));
    assert_eq!(new_text(12), "(c/get-in ((quote :x) m) [:y :z])");
    assert!(action(response(13)).is_none());
    let rewritten = new_text(11);
    for marker in [
        "#_:outer",
        "; after lookup",
        "; after receiver",
        "; before first key",
        "#_:first",
        "; after first key",
        "#_:second",
        "; after second key",
        "; after last key",
        "#_:end",
        "; after path",
    ] {
        assert_eq!(rewritten.matches(marker).count(), 1, "{rewritten}");
    }
    let forms = reader::read(rewritten).unwrap();
    assert_eq!(forms.len(), 1, "{rewritten}");
    let mut lookup = &forms[0];
    for name in [":z", ":y", ":x"] {
        let Kind::List(parts) = &lookup.kind else {
            panic!("expected a two-form lookup: {rewritten}");
        };
        let [
            Form {
                kind: Kind::List(key),
                ..
            },
            receiver,
        ] = parts.as_slice()
        else {
            panic!("expected quote operator and receiver: {rewritten}");
        };
        let [quote, keyword] = key.as_slice() else {
            panic!("expected a quoted keyword: {rewritten}");
        };
        assert_eq!(quote.symbol(), Some("quote"));
        assert_eq!(keyword.keyword(), Some(name));
        lookup = receiver;
    }
    assert_eq!(lookup.symbol(), Some("m"));
}

#[test]
fn get_in_none_refuses_dynamic_paths_defaulted_get_in_and_unproved_calls() {
    let fixture = Fixture::new("get in none refusals");
    let folder = fixture.0.join("src/demo");
    fs::create_dir(&folder).unwrap();
    fs::write(
        folder.join("fake.clj"),
        "(ns demo.fake)\n(defn get-in [m p] [m p])\n",
    )
    .unwrap();
    let source = concat!(
        "(ns demo.refusals (:require [demo.fake :as fake]))\n",
        "(defn dynamic-first [m key!] (get-in m [(key!) :x :y]))\n",
        "(defn dynamic-suffix [m key!] (get-in m [:x :y (key!)]))\n",
        "(defn dynamic-path [m path] (get-in m path))\n",
        "(defn empty-path [m] (get-in m []))\n",
        "(defn quoted-path [m] (get-in m '[:x :y]))\n",
        "(defn non-operator-key [m] (get-in m [:x 0]))\n",
        "(defn defaulted-one [m fallback!] (get-in m [:x] (fallback!)))\n",
        "(defn defaulted-several [m fallback!] (get-in m [:x :y :z] (fallback!)))\n",
        "(defn quoted [m] '(get-in m [:x :y]))\n",
        "(defn discarded [m] #_(get-in m [:x :y]) nil)\n",
        "(defn shadow [m get-in] (get-in m [:x :y]))\n",
        "(defn foreign [m] (fake/get-in m [:x :y]))\n",
        "(defmacro as-data [form] (list 'quote form))\n",
        "(defn opaque [m] (as-data (get-in m [:x :y])))\n",
        "(defn dynamic-get [m k] (get m k))\n",
    );
    let path = folder.join("refusals.clj");
    let file = uri(&path);
    fs::write(path, source).unwrap();
    let portable =
        "(ns demo.portable)\n(defn inactive [m] #?(:clj :jvm :cljs (get-in m [:x :y])))\n";
    let portable_path = folder.join("portable.cljc");
    fs::write(&portable_path, portable).unwrap();
    let mut cases: Vec<_> = (1..=11)
        .map(|line| (file.clone(), position(source, line, "(get-in")))
        .collect();
    cases.extend([
        (file.clone(), position(source, 12, "(fake/get-in")),
        (file.clone(), position(source, 14, "(get-in")),
        (file.clone(), position(source, 15, "(get m")),
        (file.clone(), position(source, 1, " (get-in")),
        (file, position(source, 1, "get-in m")),
        (uri(&portable_path), position(portable, 1, "(get-in")),
    ]);
    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
    ];
    for (index, (file, cursor)) in cases.iter().enumerate() {
        let id = 2 + index * 2;
        requests.extend([
            request_action(id, file, *cursor, "refactor.rewrite"),
            request_command(id + 1, COMMAND, &json!([file, cursor.0, cursor.1])),
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
        .replacen("['x 'y 'z]", "['new 'x 'y 'z]", 1);
    for (id, range @ (line, start, _), edit) in [
        (2, (1, 22, 43), "('z ('y ('x m)))"),
        (4, (2, 14, 34), "(:x {:x m})"),
        (6, (3, 16, 39), "('y (:x {:x m}))"),
        (8, (4, 25, 50), "('y ((quote x) m))"),
        (10, (5, 22, 37), "(:x {:x m})"),
        (12, (6, 39, 60), "(:x m (default!))"),
        (14, (7, 38, 59), "('x m (default!))"),
        (16, (8, 30, 47), "((quote x) m)"),
    ] {
        let (reply, _) = exchange(request_action(id, &file, (line, start), "refactor"), None);
        let action = action(&reply).unwrap();
        assert_eq!(action, &expected_action(&file, (line, start)));
        let (reply, applied) = exchange(
            request_command(id + 1, COMMAND, &action["command"]["arguments"]),
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
    let expected = workspace_edit(&file, 7, (1, 24, 50), "('z ('y ('x ('new m))))");
    for (id, accepted) in [(19, false), (20, true)] {
        let (reply, applied) = exchange(
            request_command(id, COMMAND, &action["command"]["arguments"]),
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

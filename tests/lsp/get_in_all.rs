use super::*;
use parenox_analysis::reader::{self, Form, Kind};

const COMMAND: &str = "get-in-all";
const TITLE: &str = "Move all expressions to get/get-in";
const SOURCE: &str = concat!(
    "(ns demo.lookups)\n",
    "(defn nesting [m] [\"🧪\" (:z (:y (:x m)))])\n",
    "(defn single [m] (:key m))\n",
    "(defn mixed-receiver [m] (get (:a m) :b))\n",
    "(defn existing-path [m] (get-in (:a m) [:b :c]))\n",
);
const OVERLAY: &str = concat!(
    "(ns demo.lookups)\n",
    "(defn nesting [m] [\"🧪🦀\" (:new (:z (:y (:x m))))])\n",
    "(defn single [m] (:key m))\n",
    "(defn mixed-receiver [m] (get (:a m) :b))\n",
    "(defn existing-path [m] (get-in (:a m) [:b :c]))\n",
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
fn get_in_all_preserves_utf16_ranges_and_uses_the_live_buffer_version() {
    let fixture = Fixture::new("get in all");
    verified_transcript(lsp_server(), &fixture.0, false);
}

#[test]
#[ignore = "requires a Clojure LSP executable"]
fn get_in_all_matches_pinned_reference_for_safe_lookup_chains() {
    let fixture = Fixture::new("get in all parity");
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
fn get_in_all_requires_proven_lookups_and_retains_opaque_receivers_and_trivia() {
    let fixture = Fixture::new("get in all guards");
    let path = fixture.0.join("src/guards.cljc");
    let file = uri(&path);
    let source = concat!(
        "(ns demo.lookups (:require [clojure.core :as c]))\n",
        "(defn explicit [m] (get (get m :a) :b))\n",
        "(defn inner-path [m] (get (get-in m [:a :b]) :c))\n",
        "(defn explicit-inner [m] (:z (get (:a m) :b)))\n",
        "(defn vector-key [m] (get (:a m) [:b :c]))\n",
        "(defn qualified [m] (clojure.core/get (:a m) :b))\n",
        "(defn aliased [m] (c/get (:a m) :b))\n",
        "(defn output-shadow [get-in m] (:b (:a m)))\n",
        "(defn inner-default [m] (:z (:y (:x m :missing))))\n",
        "(defn inner-dynamic-path [m path] (:z (:y (get-in m path))))\n",
        "(defn opaque-receiver [m] (:z (:y (identity m))))\n",
        "(defn inner-shadow [get m] (:z (:y (get m :a))))\n",
        "(defn outer-default [m] (get (:a m) :b :fallback))\n",
        "(defn keyword-default [m] (:z (:y (:x m)) :fallback))\n",
        "(defn path-default [m] (get-in (:a m) [:b :c] :fallback))\n",
        "(defn dynamic-path [m path] (get-in (:a m) path))\n",
        "(defn opaque-outer [m] (identity (:a m)))\n",
        "(defn outer-shadow [get m] (get (:a m) :b))\n",
        "(defn quoted [m] '(:b (:a m)))\n",
        "(defn discarded [m] #_(:b (:a m)) nil)\n",
        "\n",
        "(defn trivia [m]\n",
        "  (get-in ; outer head\n",
        "    (:a #_:discarded ; inner receiver\n",
        "      m) ; between receiver and path\n",
        "    [ ; path start\n",
        "      :b #_:inside ; between keys\n",
        "      :c] ; trailing\n",
        "    ))\n",
        "(defn dynamic-key [m key] (get (:a m) key))\n",
        "(defn dynamic-vector-key [m key] (get-in (:a m) [key]))\n",
        "(defn preserve-aliased [m] (c/get-in (:a m) [:b :c]))\n",
        "(defn preserve-qualified [m] (clojure.core/get-in (:a m) [:b :c]))\n",
        "(defn destructured [{:keys [:example/get-in]} m] (:b (:a m)))\n",
        "(defn portable [m] #?(:clj :jvm :cljs (:b (:a m))))\n",
    );
    fs::write(&path, source).unwrap();
    let cases = [
        (1, "(get", Some("(get-in m [:a :b])")),
        (2, "(get", Some("(get-in m [:a :b :c])")),
        (3, "(:z", Some("(get-in m [:a :b :z])")),
        (4, "(get", Some("(get-in m [:a [:b :c]])")),
        (5, "(clojure.core/get", Some("(get-in m [:a :b])")),
        (6, "(c/get", Some("(get-in m [:a :b])")),
        (7, "(:b", Some("(clojure.core/get-in m [:a :b])")),
        (8, "(:z", Some("(get-in (:x m :missing) [:y :z])")),
        (9, "(:z", Some("(get-in (get-in m path) [:y :z])")),
        (10, "(:z", Some("(get-in (identity m) [:y :z])")),
        (11, "(:z", Some("(get-in (get m :a) [:y :z])")),
        (12, "(get", None),
        (13, "(:z", None),
        (14, "(get-in", None),
        (15, "(get-in", None),
        (16, "(identity", None),
        (17, "(get", None),
        (18, "(:b", None),
        (19, "(:b", None),
        (7, " (:b", None),
        (1, "get (get", None),
        (29, "(get", None),
        (30, "(get-in", None),
        (31, "(c/get-in", Some("(c/get-in m [:a :b :c])")),
        (
            32,
            "(clojure.core/get-in",
            Some("(clojure.core/get-in m [:a :b :c])"),
        ),
        (33, "(:b", Some("(clojure.core/get-in m [:a :b])")),
        (34, "(:b", None),
    ];
    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":file,"version":1,"languageId":"clojure","text":source
        }}}),
    ];
    for (index, (line, needle, _)) in cases.iter().enumerate() {
        let row = source.lines().nth(*line).unwrap();
        let character = row[..row.find(needle).unwrap()].encode_utf16().count();
        let id = 2 + index * 2;
        requests.extend([
            request_action(id, &file, (*line, character), "refactor.rewrite"),
            request_command(id + 1, &json!([file, line, character])),
        ]);
    }
    let trivia_id = 2 + cases.len() * 2;
    requests.extend([
        request_action(trivia_id, &file, (22, 2), "refactor.rewrite"),
        request_command(trivia_id + 1, &json!([file, 22, 2])),
        request_action(trivia_id + 2, &file, (1, 0), "refactor.extract"),
        json!({"jsonrpc":"2.0","id":trivia_id+3,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let replies = transcript(&requests);
    let response = |id| replies.iter().find(|message| message["id"] == id).unwrap();
    let rewritten = |id| {
        response(id)["result"]["changes"][&file][0]["newText"]
            .as_str()
            .unwrap_or_else(|| panic!("expected edit: {}", response(id)))
    };
    for (index, (line, needle, expected)) in cases.into_iter().enumerate() {
        let id = 2 + index * 2;
        if let Some(expected) = expected {
            assert!(action(response(id)).is_some(), "{line}: {needle}");
            assert_eq!(rewritten(id + 1), expected, "{line}: {needle}");
        } else {
            assert!(action(response(id)).is_none(), "{line}: {needle}");
            assert_eq!(
                response(id + 1)["error"],
                json!({"code":-32600,"message":"Nothing to change."}),
                "{line}: {needle}"
            );
        }
    }
    assert!(action(response(trivia_id)).is_some());
    assert!(action(response(trivia_id + 2)).is_none());
    let text = rewritten(trivia_id + 1);
    for marker in [
        "; outer head",
        "#_:discarded",
        "; inner receiver",
        "; between receiver and path",
        "; path start",
        "#_:inside",
        "; between keys",
        "; trailing",
    ] {
        assert_eq!(text.matches(marker).count(), 1, "{text}");
    }
    let forms = reader::read(text).unwrap();
    let [
        Form {
            kind: Kind::List(items),
            ..
        },
    ] = forms.as_slice()
    else {
        panic!("expected one lookup form: {text}");
    };
    let [head, receiver, lookup_path] = items.as_slice() else {
        panic!("expected get-in with receiver and one path: {text}");
    };
    assert_eq!(head.symbol(), Some("get-in"));
    assert_eq!(receiver.symbol(), Some("m"));
    assert_eq!(
        lookup_path
            .vector()
            .unwrap()
            .iter()
            .map(Form::keyword)
            .collect::<Vec<_>>(),
        [Some(":a"), Some(":b"), Some(":c")]
    );
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
fn get_in_all_refuses_opaque_macros_and_uncertain_portable_bindings() {
    let fixture = Fixture::new("get in all provenance");
    let directory = fixture.0.join("src/demo");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("fake.clj"),
        "(ns demo.fake)\n(defn get [m k] [m k])\n",
    )
    .unwrap();
    let fixtures = [
        (
            "quoted_macro.clj",
            "(ns demo.quoted-macro)\n(defmacro as-data [form] (list 'quote form))\n(defn f [m] (as-data (:b (:a m))))\n",
            2,
            "(:b",
        ),
        (
            "rebound_core.clj",
            "(ns demo.rebound-core (:require [demo.fake :as clojure.core]))\n(defn f [m] (:b (:a m)))\n",
            1,
            "(:b",
        ),
        (
            "dynamic_core.clj",
            "(ns demo.dynamic-core (:require [demo.fake]))\n(alias 'clojure.core 'demo.fake)\n(def f (fn* ([get m] (:b m))))\n",
            2,
            "(:b",
        ),
        (
            "nested_require.clj",
            "(ns demo.nested-require)\n(do (require '[demo.fake :as clojure.core]))\n(def f (fn* ([m] (:b (:a m)))))\n",
            2,
            "(:b",
        ),
        (
            "portable_core.cljc",
            "(ns demo.portable-core (:require [demo.fake :as cljs.core]))\n(def f (fn* ([get-in m] (:b (:a m)))))\n",
            1,
            "(:b",
        ),
        (
            "portable_namespace.cljc",
            "(ns demo.portable-namespace #?(:clj (:require [clojure.core :as c]) :cljs (:require [demo.fake :as c])))\n(defn f [m] (c/get (:a m) :b))\n",
            1,
            "(c/get",
        ),
        (
            "portable_binding.cljc",
            "(ns demo.portable-binding)\n(defn f [#?(:clj ignored :cljs get-in) m] (:b (:a m)))\n",
            1,
            "(:b",
        ),
    ];
    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
    ];
    for (index, (name, source, line, needle)) in fixtures.iter().enumerate() {
        let path = directory.join(name);
        fs::write(&path, source).unwrap();
        let file = uri(&path);
        let row = source.lines().nth(*line).unwrap();
        let character = row[..row.find(needle).unwrap()].encode_utf16().count();
        let id = 2 + index * 2;
        requests.extend([
            request_action(id, &file, (*line, character), "refactor.rewrite"),
            request_command(id + 1, &json!([file, line, character])),
        ]);
    }
    requests.extend([
        json!({"jsonrpc":"2.0","id":2 + fixtures.len() * 2,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let replies = transcript(&requests);
    let response = |id| replies.iter().find(|message| message["id"] == id).unwrap();
    for (index, (name, _, _, _)) in fixtures.iter().enumerate() {
        let id = 2 + index * 2;
        assert!(action(response(id)).is_none(), "{name}: {}", response(id));
        assert_eq!(
            response(id + 1)["error"],
            json!({"code":-32600,"message":"Nothing to change."}),
            "{name}"
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
    for (id, range @ (line, start, _), edit) in [
        (2, (1, 24, 40), "(get-in m [:x :y :z])"),
        (4, (2, 17, 25), "(get m :key)"),
        (6, (3, 25, 40), "(get-in m [:a :b])"),
        (8, (4, 24, 47), "(get-in m [:a :b :c])"),
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
            "textDocument":{"uri":file,"version":7},"contentChanges":[{"text":OVERLAY}]
        }}),
        None,
    );
    let cursor = (1, 26);
    let (reply, _) = exchange(request_action(10, &file, cursor, "refactor.rewrite"), None);
    let action = action(&reply).unwrap();
    assert_eq!(action, &expected_action(&file, cursor));
    let expected = workspace_edit(&file, 7, (1, 26, 49), "(get-in m [:x :y :z :new])");
    for (id, accepted) in [(11, false), (12, true)] {
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
    exchange(json!({"jsonrpc":"2.0","id":13,"method":"shutdown"}), None);
    exchange(json!({"jsonrpc":"2.0","method":"exit"}), None);
    drop(input);
    assert!(child.wait().unwrap().success());
    assert_eq!(fs::read_to_string(path).unwrap(), SOURCE);
    matched
}

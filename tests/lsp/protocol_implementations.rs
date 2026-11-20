use super::*;

const SHAPE: &str = concat!(
    "(ns lab.shape)\n",
    "(defprotocol Measure\n",
    "  \"Measuring values.\"\n",
    "  (measure [value] [value factor] \"Measure a value.\")\n",
    "  (name-of [value]))\n",
    "(defn use-local [value] (measure value))\n",
);
const PEER: &str = "(ns lab.peer)\n(defprotocol Measure\n  (measure [value]))\n";
const IMPLS: &str = concat!(
    "(ns lab.impls\n",
    "  (:require [lab.shape :as p :refer [measure]]\n",
    "            [lab.peer :as q]\n",
    "            [clojure.core :as c]))\n",
    "\n",
    "(defrecord ^{:doc \"🧪\"} Record [n] p/Measure (measure [_] n) (measure [_ factor] (* n factor))\n",
    "  (name-of [_] :record)\n",
    "  Object\n",
    "  (toString [_] \"record\"))\n",
    "\n",
    "(deftype Inline [n]\n",
    "  p/Measure\n",
    "  (measure [_] n)\n",
    "  (measure [_ factor] (* n factor))\n",
    "  (name-of [_] :inline))\n",
    "\n",
    "(extend-type String\n",
    "  p/Measure\n",
    "  (measure ([s] (.length s)) ([s f] (* (.length s) f)))\n",
    "  (name-of [_] :string)\n",
    "  q/Measure\n",
    "  (measure [_] :peer-string))\n",
    "\n",
    "(extend-protocol p/Measure\n",
    "  Long\n",
    "  (measure ([x] x) ([x f] (* x f)))\n",
    "  (name-of [_] :long)\n",
    "  nil\n",
    "  (measure ([_] :none) ([_ _] :none))\n",
    "  (name-of [_] :nil))\n",
    "\n",
    "(c/extend-type Boolean\n",
    "  p/Measure\n",
    "  (measure ([_] :boolean) ([_ _] :boolean))\n",
    "  (name-of [_] :boolean))\n",
    "\n",
    "(defrecord Peer []\n",
    "  q/Measure\n",
    "  (measure [_] :peer-type))\n",
);
const SHADOW: &str = concat!(
    "(ns lab.shadow\n",
    "  (:refer-clojure :exclude [extend-type])\n",
    "  (:require [lab.shape :as p]))\n",
    "\n",
    "(defn extend-type [& xs] xs)\n",
    "(extend-type Number p/Measure (measure [_] :not-an-implementation))\n",
    "(clojure.core/extend-type Character p/Measure\n",
    "  (measure ([_] :char) ([_ _] :char))\n",
    "  (name-of [_] :char))\n",
);
const CALLER: &str = concat!(
    "(ns lab.caller (:require [lab.shape :as p :refer [measure]] [lab.peer :as q]))\n",
    "(defn usage [value] [\"🧪\" (p/measure value) (measure value) (q/measure value)])\n",
    "(defn shadow [measure value] (measure value))\n",
    "(defn ordinary [value] value)\n",
    "(ordinary :input)\n",
    "(lab.shape/measure nil)\n",
    "(p/measure nil)\n",
    "'(p/measure nil)\n",
    "(p/missing nil)\n",
);
const NESTED_REIFY: &str = concat!(
    "(ns lab.nested\n",
    "  (:require [lab.shape :as p] [lab.peer :as q] [clojure.core :as c]))\n",
    "(defn ^{:doc \"🧪\"} direct [] (reify p/Measure (measure [_] :one) (measure [_ _] :two) (name-of [_] :direct)))\n",
    "(defn via-let [flag]\n",
    "  (let [local (reify p/Measure\n",
    "                (measure [_] :binding)\n",
    "                (measure [_ _] :binding)\n",
    "                (name-of [_] :binding))]\n",
    "    (if flag\n",
    "      (reify p/Measure\n",
    "        (measure [_] :branch)\n",
    "        (measure [_ _] :branch)\n",
    "        (name-of [_] :branch))\n",
    "      local)))\n",
    "(def initial (clojure.core/reify p/Measure\n",
    "  (measure [_] :initial)\n",
    "  (measure [_ _] :initial)\n",
    "  (name-of [_] :initial)))\n",
    "(defn qualified [reify] (c/reify p/Measure\n",
    "  (measure [_] :qualified)\n",
    "  (measure [_ _] :qualified)\n",
    "  (name-of [_] :qualified)))\n",
    "(def peer (reify q/Measure (measure [_] :peer)))\n",
);

#[test]
fn protocol_implementation_resolves_named_methods_and_tracks_live_locations() {
    let fixture = Fixture::new("protocol implementation");
    let root = fixture.0.canonicalize().unwrap();
    write_project(&root);
    verified_transcript(lsp_server(), &root);
}

#[test]
fn protocol_implementation_tracks_only_executable_nested_reify() {
    let fixture = Fixture::new("nested reify implementation");
    let folder = fixture.0.join("src/lab");
    fs::create_dir(&folder).unwrap();
    let shape = uri(&folder.join("shape.clj"));
    let peer = uri(&folder.join("peer.clj"));
    let nested_path = folder.join("nested.clj");
    let nested = uri(&nested_path);
    fs::write(folder.join("shape.clj"), SHAPE).unwrap();
    fs::write(folder.join("peer.clj"), PEER).unwrap();
    let source = format!(
        "{NESTED_REIFY}{}",
        concat!(
            "(defn quoted [] '(reify p/Measure (measure [_] :quoted)))\n",
            "(defn commented [] (comment (reify p/Measure (measure [_] :commented))))\n",
            "(defn shadow [reify] (reify p/Measure (measure [_] :local)))\n",
            "(defn unresolved [] (opaque (reify p/Measure (measure [_] :opaque))))\n",
            "(defn discarded [] #_(reify p/Measure (measure [_] :discarded)) nil)\n",
        )
    );
    fs::write(&nested_path, &source).unwrap();
    let live_start = source.lines().count() + 1;
    let overlay = format!(
        ";; unsaved offset change\n{source}(def live (reify p/Measure\n  (measure [_] :live)\n  (measure [_ _] :live)\n  (name-of [_] :live)))\n"
    );
    let query = |id, file, line, column| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/implementation","params":{
            "textDocument":{"uri":file},"position":{"line":line,"character":column}
        }})
    };
    let replies = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        query(2, &shape, 3, 6),
        query(3, &shape, 4, 6),
        query(4, &peer, 2, 6),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":nested,"languageId":"clojure","version":1,"text":source
        }}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":nested,"version":7},"contentChanges":[{"text":overlay}]
        }}),
        query(5, &shape, 3, 6),
        query(6, &shape, 4, 6),
        query(7, &peer, 2, 6),
        json!({"jsonrpc":"2.0","id":8,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| {
        let reply = replies.iter().find(|message| message["id"] == id).unwrap();
        assert!(reply["error"].is_null(), "{reply}");
        locations(reply["result"].clone())
    };
    let measure: [(usize, usize, usize); 10] = [
        (2, 47, 54),
        (2, 66, 73),
        (5, 17, 24),
        (6, 17, 24),
        (10, 9, 16),
        (11, 9, 16),
        (15, 3, 10),
        (16, 3, 10),
        (19, 3, 10),
        (20, 3, 10),
    ];
    let name_of = [
        (2, 87, 94),
        (7, 17, 24),
        (12, 9, 16),
        (17, 3, 10),
        (21, 3, 10),
    ];
    for (id, spans, moved, added) in [
        (2, &measure[..], 0, &[][..]),
        (3, &name_of[..], 0, &[][..]),
        (4, &[(22, 28, 35)][..], 0, &[][..]),
        (5, &measure[..], 1, &[live_start + 1, live_start + 2][..]),
        (6, &name_of[..], 1, &[live_start + 3][..]),
        (7, &[(22, 28, 35)][..], 1, &[][..]),
    ] {
        let mut expected: Vec<_> = spans
            .iter()
            .map(|(line, start, end)| location(&nested, line + moved, *start, *end))
            .collect();
        expected.extend(added.iter().map(|line| location(&nested, *line, 3, 10)));
        expected.sort_by_cached_key(Value::to_string);
        assert_eq!(result(id), expected, "request {id}");
    }
    assert_eq!(fs::read_to_string(nested_path).unwrap(), source);
}

#[test]
#[ignore = "requires a Clojure LSP executable"]
fn protocol_implementation_matches_pinned_reference() {
    let fixture = Fixture::new("protocol implementation parity");
    let root = fixture.0.canonicalize().unwrap();
    write_project(&root);
    let binary = std::env::var_os("PARENOX_REFERENCE_LSP").unwrap_or_else(|| "clojure-lsp".into());
    let reference = Command::new(binary)
        .arg("listen")
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let binary = std::env::var_os("PARENOX_LSP_CANDIDATE")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_parenox").into());
    assert_eq!(
        verified_transcript(reference, &root),
        verified_transcript(lsp_server_with(Path::new(&binary)), &root)
    );
}

fn write_project(root: &Path) {
    let folder = root.join("src/lab");
    fs::create_dir(&folder).unwrap();
    for (name, source) in [
        ("shape", SHAPE),
        ("peer", PEER),
        ("impls", IMPLS),
        ("shadow", SHADOW),
        ("caller", CALLER),
    ] {
        fs::write(folder.join(format!("{name}.clj")), source).unwrap();
    }
}

fn location(file: &str, line: usize, start: usize, end: usize) -> Value {
    json!({"uri":file,"range":{
        "start":{"line":line,"character":start},"end":{"line":line,"character":end}
    }})
}

fn verified_transcript(mut child: Child, root: &Path) -> Vec<Vec<Value>> {
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut request_id = 0;
    let mut exchange = |method: &str, params: Value, request: bool| {
        let mut message = json!({"jsonrpc":"2.0","method":method,"params":params});
        if request {
            request_id += 1;
            message["id"] = json!(request_id);
        }
        send_message(&mut input, &message);
        if !request {
            return Value::Null;
        }
        loop {
            let reply = read_message(&mut output).expect("LSP message");
            if reply["id"] == request_id && reply["method"].is_null() {
                assert!(reply["error"].is_null(), "{reply}");
                return reply["result"].clone();
            }
            if !reply["method"].is_null() && !reply["id"].is_null() {
                send_message(
                    &mut input,
                    &json!({"jsonrpc":"2.0","id":reply["id"],"result":null}),
                );
            }
        }
    };
    let initialize = exchange(
        "initialize",
        json!({"rootUri":uri(root),"capabilities":{}}),
        true,
    );
    assert_eq!(initialize["capabilities"]["implementationProvider"], true);
    exchange("initialized", json!({}), false);
    let file = |name: &str| uri(&root.join(format!("src/lab/{name}.clj")));
    for (name, source) in [("shape", SHAPE), ("caller", CALLER)] {
        exchange(
            "textDocument/didOpen",
            json!({"textDocument":{
                "uri":file(name),"languageId":"clojure","version":1,"text":source
            }}),
            false,
        );
    }
    let spans: [(usize, usize, usize); 8] = [
        (5, 46, 53),
        (5, 62, 69),
        (12, 3, 10),
        (13, 3, 10),
        (18, 3, 10),
        (25, 3, 10),
        (28, 3, 10),
        (33, 3, 10),
    ];
    let measure = |shift| {
        let mut locations = vec![location(&file("shadow"), 7, 3, 10)];
        locations.extend(
            spans.map(|(line, start, end)| location(&file("impls"), line + shift, start, end)),
        );
        locations
    };
    let mut measured = measure(0);
    let mut peer = vec![
        location(&file("impls"), 21, 3, 10),
        location(&file("impls"), 38, 3, 10),
    ];
    let mut named = vec![location(&file("shadow"), 8, 3, 10)];
    named.extend([6, 14, 19, 26, 29, 34].map(|line| location(&file("impls"), line, 3, 10)));
    let mut live = measure(2);
    live.push(location(&file("impls"), 44, 3, 10));
    for items in [&mut measured, &mut peer, &mut named, &mut live] {
        items.sort_by_cached_key(Value::to_string);
    }
    let empty = Vec::new();
    let mut captured = Vec::new();
    for (name, source, line, word, offset, expected) in [
        ("shape", SHAPE, 3, "measure", 4, &measured),
        ("shape", SHAPE, 4, "name-of", 4, &named),
        ("shape", SHAPE, 5, "(measure", 4, &measured),
        ("peer", PEER, 2, "measure", 4, &peer),
        ("caller", CALLER, 0, "[measure]", 5, &measured),
        ("caller", CALLER, 1, "p/measure", 0, &measured),
        ("caller", CALLER, 1, "p/measure", 5, &measured),
        ("caller", CALLER, 1, "(measure value", 4, &measured),
        ("caller", CALLER, 1, "q/measure", 5, &peer),
        ("caller", CALLER, 2, "(measure value", 4, &empty),
        ("caller", CALLER, 4, "ordinary", 4, &empty),
        ("caller", CALLER, 5, "lab.shape/measure", 13, &measured),
        ("caller", CALLER, 6, "p/measure", 5, &measured),
        ("caller", CALLER, 7, "p/measure", 5, &empty),
        ("caller", CALLER, 8, "p/missing", 4, &empty),
    ] {
        let row = source.lines().nth(line).unwrap();
        let character = row[..row.find(word).unwrap()].encode_utf16().count() + offset;
        let actual = locations(exchange(
            "textDocument/implementation",
            json!({
                "textDocument":{"uri":file(name)},"position":{"line":line,"character":character}
            }),
            true,
        ));
        assert_eq!(&actual, expected, "{name} {line}:{character}");
        captured.push(actual);
    }

    exchange(
        "textDocument/didOpen",
        json!({"textDocument":{
            "uri":file("impls"),"languageId":"clojure","version":1,"text":IMPLS
        }}),
        false,
    );
    let overlay = format!(
        ";; unsaved\n;; offsets moved\n{IMPLS}\n(extend-type Double\n  p/Measure\n  (measure ([x] x) ([x f] (* x f)))\n  (name-of [_] :double))\n"
    );
    exchange(
        "textDocument/didChange",
        json!({
            "textDocument":{"uri":file("impls"),"version":7},"contentChanges":[{"text":overlay}]
        }),
        false,
    );
    let mut updated = Vec::new();
    for _ in 0..20 {
        updated = locations(exchange(
            "textDocument/implementation",
            json!({
                "textDocument":{"uri":file("shape")},"position":{"line":3,"character":6}
            }),
            true,
        ));
        if updated == live {
            break;
        }
        // Clojure LSP indexes document changes asynchronously.
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(updated, live, "after didChange v7");
    captured.push(updated);
    exchange("shutdown", Value::Null, true);
    exchange("exit", Value::Null, false);
    drop(input);
    assert!(child.wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(root.join("src/lab/impls.clj")).unwrap(),
        IMPLS
    );
    captured
}

fn locations(value: Value) -> Vec<Value> {
    let mut items = value
        .as_array()
        .unwrap_or_else(|| panic!("expected an array of locations: {value}"))
        .clone();
    items.sort_by_cached_key(Value::to_string);
    assert!(
        items.windows(2).all(|pair| pair[0] != pair[1]),
        "duplicate locations: {items:?}"
    );
    items
}

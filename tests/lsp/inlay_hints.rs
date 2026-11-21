use super::*;

#[test]
fn inlay_hints_show_reached_bindings_once_and_follow_unsaved_types() {
    let fixture = Fixture::new("inlay hints");
    let file = fixture.0.join("src/types.clj");
    let document = uri(&file);
    let source = r#"(ns types (:require [clojure.spec.alpha :as s] [clojure.string :as str]))
(defn render {:t/sig '[(s/nilable string?) :-> string?]} [input]
  (if (nil? input)
    "default"
    (let [#_"😀" exact (str/upper-case input)]
      exact)))
(defn independent {:t/sig '[string? :-> string?]} [value]
  (if false
    (let [dead value] dead)
    (let [reached value] reached)))
"#;
    let invalid = source.replace("(nil? input)", "(some? input)");
    fs::write(&file, source).unwrap();
    let rows: Vec<_> = source.lines().collect();
    let binding_position = |line: usize, name: &str| {
        let end = rows[line].find(name).unwrap() + name.len();
        json!({"line":line,"character":rows[line][..end].encode_utf16().count()})
    };
    let exact = binding_position(4, "exact");
    let reached = binding_position(9, "reached");
    let first = json!({"line":4,"character":0});
    let last = json!({"line":10,"character":0});
    let request = |id, start: &Value, end: &Value| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/inlayHint","params":{
            "textDocument":{"uri":document},"range":{"start":start,"end":end}
        }})
    };
    let change = |version, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let replies = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":document,"languageId":"clojure","version":1,"text":source}
        }}),
        request(2, &first, &last),
        request(3, &first, &exact),
        request(4, &exact, &json!({"line":5,"character":0})),
        change(2, &invalid),
        request(5, &first, &last),
        change(3, source),
        request(6, &first, &last),
        json!({"jsonrpc":"2.0","id":7,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| &replies.iter().find(|reply| reply["id"] == id).unwrap()["result"];
    assert_eq!(result(1)["capabilities"]["inlayHintProvider"], true);
    let exact = json!({"position":exact,"kind":1,"label":": String"});
    let parameter = json!({"position":binding_position(6, "value"),"kind":1,"label":": String"});
    let reached = json!({"position":reached,"kind":1,"label":": String"});
    assert_eq!(result(2), &json!([exact, parameter, reached]));
    assert_eq!(result(3), &json!([]));
    assert_eq!(result(4), &json!([exact]));
    assert_eq!(result(5), &json!([parameter, reached]));
    assert_eq!(result(2), result(6));
    let errors = |version| {
        replies
            .iter()
            .filter(|reply| {
                reply["method"] == "textDocument/publishDiagnostics"
                    && reply["params"]["version"] == version
            })
            .flat_map(|reply| reply["params"]["diagnostics"].as_array().unwrap())
            .filter(|diagnostic| diagnostic["code"] == "PNX2322")
            .count()
    };
    assert_eq!([errors(1), errors(2), errors(3)], [0, 1, 0]);
}

#[test]
fn inlay_hints_withhold_escaping_callback_types_and_keep_known_local_results() {
    let fixture = Fixture::new("inlay hints and escaping callbacks");
    let file = fixture.0.join("src/types.clj");
    let document = uri(&file);
    let source = r#"(ns types)
(defn escapes {:t/sig '[:-> t/any]} []
  (letfn [(inner [nested] nested) (outward [captured] (inner captured))]
    (outward 1)
    outward))
(defn incompatible {:t/sig '[:-> string?]} [] (let [wrong 1] wrong))
"#;
    fs::write(&file, source).unwrap();
    let replies = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":document,"languageId":"clojure","version":1,"text":source}
        }}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/inlayHint","params":{
            "textDocument":{"uri":document},"range":{
                "start":{"line":1,"character":0},"end":{"line":6,"character":0}
            }
        }}),
        json!({"jsonrpc":"2.0","id":3,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let inlay = replies.iter().find(|reply| reply["id"] == 2).unwrap();
    let row = source.lines().nth(5).unwrap();
    let end = row.find("wrong").unwrap() + "wrong".len();
    assert_eq!(
        inlay["result"],
        json!([
            {"position":{"line":5,"character":end},"label":": 1","kind":1}
        ])
    );
    let publish = replies
        .iter()
        .find(|reply| reply["method"] == "textDocument/publishDiagnostics")
        .unwrap();
    let diagnostics = publish["params"]["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == "PNX9001")
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == "PNX2322")
    );
}

#[test]
fn saved_extern_changes_refresh_inlay_hints_with_one_deferred_request() {
    let fixture = Fixture::new("inlay hint refresh");
    let file = fixture.0.join("src/types.clj");
    let externs = fixture.0.join("externs.edn");
    let source = "(ns types (:require [fixture.api :as api]))\n\
        (defn render {:t/sig '[:-> t/any]} []\n  \
        (let [result (api/normalize \"input\")] result))\n";
    fs::write(&file, source).unwrap();
    fs::write(
        fixture.0.join("parenox.edn"),
        r#"{:externs ["externs.edn"]}"#,
    )
    .unwrap();
    let contract = |result| {
        fs::write(
            &externs,
            format!("{{fixture.api/normalize [[string? :-> {result}]]}}"),
        )
        .unwrap();
    };
    contract("string?");
    let mut child = lsp_server();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut exchange = |request: Value, before: &[Value]| {
        for frame in before {
            send_message(&mut input, frame);
        }
        send_message(&mut input, &request);
        let mut frames = Vec::new();
        loop {
            let reply = read_message(&mut output).expect("LSP response");
            if reply["id"] == request["id"] {
                assert!(reply.get("error").is_none(), "{reply}");
                break (reply["result"].clone(), frames);
            }
            frames.push(reply);
        }
    };
    exchange(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&fixture.0),
            "capabilities":{"workspace":{"inlayHint":{"refreshSupport":true}}}
        }}),
        &[],
    );
    let document = uri(&file);
    let request = |id| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/inlayHint","params":{
            "textDocument":{"uri":document},"range":{
                "start":{"line":2,"character":0},"end":{"line":3,"character":0}
            }
        }})
    };
    let save = json!({"jsonrpc":"2.0","method":"textDocument/didSave","params":{
        "textDocument":{"uri":uri(&externs)}
    }});
    let watch = json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles","params":{
        "changes":[{"uri":uri(&externs),"type":2}]
    }});
    let (initial, _) = exchange(
        request(2),
        &[
            json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
            json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
                "textDocument":{"uri":document,"languageId":"clojure","version":1,"text":source}
            }}),
        ],
    );
    assert_eq!(initial[0]["label"], ": String");
    contract("number?");
    let (number, frames) = exchange(request(3), std::slice::from_ref(&watch));
    assert_eq!(number[0]["label"], ": Number");
    let refreshes = |frames: Vec<Value>| -> Vec<Value> {
        frames
            .into_iter()
            .filter(|frame| frame["method"] == "workspace/inlayHint/refresh")
            .map(|frame| {
                assert!(frame.get("params").is_none(), "{frame}");
                assert!(frame["id"].is_string(), "{frame}");
                frame["id"].clone()
            })
            .collect()
    };
    let first = refreshes(frames);
    assert_eq!(first.len(), 1);
    let (_, frames) = exchange(request(4), std::slice::from_ref(&save));
    assert!(refreshes(frames).is_empty());
    contract("boolean?");
    let (boolean, frames) = exchange(request(5), std::slice::from_ref(&save));
    assert_eq!(boolean[0]["label"], ": Boolean");
    assert!(refreshes(frames).is_empty());
    contract("string?");
    let (restored, frames) = exchange(request(6), std::slice::from_ref(&watch));
    assert_eq!(restored, initial);
    assert!(refreshes(frames).is_empty());
    let ack = |id: &Value| json!({"jsonrpc":"2.0","id":id,"result":null});
    let (_, frames) = exchange(request(7), &[ack(&first[0])]);
    assert_eq!(frames.len(), 1);
    let second = refreshes(frames);
    assert_eq!(second.len(), 1);
    assert_ne!(first, second);
    let (_, frames) = exchange(request(8), &[ack(&second[0]), watch]);
    assert!(frames.is_empty());
    exchange(json!({"jsonrpc":"2.0","id":9,"method":"shutdown"}), &[]);
    send_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
}

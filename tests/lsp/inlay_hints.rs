use super::*;

#[test]
fn unannotated_functions_show_proven_bindings_in_the_requested_arity() {
    let fixture = Fixture::new("unannotated inlay");
    let file = fixture.0.join("src/types.clj");
    let document = uri(&file);
    let provider = fixture.0.join("src/fixture/api.clj");
    fs::create_dir(provider.parent().unwrap()).unwrap();
    fs::write(&provider, "(ns fixture.api)\n(defn copy [text] text)\n").unwrap();
    let provider = uri(&provider);
    let source = r#"(ns types (:require [fixture.api :as api]))
(defn render
  ([input]
   (if (string? input)
     (let [#_"😀" rendered (api/copy input)] rendered)
     nil))
  ([input suffix]
   (let [constant "suffix"] constant)))
"#;
    fs::write(&file, source).unwrap();
    fs::write(fixture.0.join("parenox.edn"), r#"{:externs ["api.edn"]}"#).unwrap();
    fs::write(
        fixture.0.join("api.edn"),
        "{fixture.api/copy [[string? :-> string?]]}",
    )
    .unwrap();
    let broken = source.replace("(api/copy input)", "(api/copy 42)");
    let alias_only = source.replace(":as api", ":as-alias api");
    let duplicate = format!(
        "{source}(defn render {{:t/sig :invalid}} [] (let [failed \"duplicate\"] failed))\n"
    );
    let update = |version, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let range = |id, start, end| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/inlayHint","params":{
            "textDocument":{"uri":document},"range":{
                "start":{"line":start,"character":0},"end":{"line":end,"character":0}
            }
        }})
    };
    let replies = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"languageId":"clojure","version":1,"text":source
        }}}),
        range(2, 2, 6),
        range(3, 6, 8),
        update(2, &broken),
        range(4, 2, 8),
        update(3, &alias_only),
        range(5, 2, 6),
        update(4, source),
        range(6, 2, 8),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":provider,"version":1,"languageId":"clojure",
            "text":"(ns fixture.api)\n(defn copy [] \"changed\")\n"
        }}}),
        range(7, 2, 8),
        json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{
            "textDocument":{"uri":provider}
        }}),
        range(8, 2, 8),
        update(5, &duplicate),
        range(9, 8, 9),
        json!({"jsonrpc":"2.0","id":10,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let at = |line: usize, name: &str| {
        let row = source.lines().nth(line).unwrap();
        let end = row.find(name).unwrap() + name.len();
        json!({"line":line,"character":row[..end].encode_utf16().count()})
    };
    let rendered = json!({"position":at(4, "rendered"),"kind":1,"label":": String"});
    let constant = json!({"position":at(7, "constant"),"kind":1,"label":": \"suffix\""});
    let result = |id| &replies.iter().find(|reply| reply["id"] == id).unwrap()["result"];
    assert_eq!(result(2), &json!([rendered]));
    assert_eq!(result(3), &json!([constant]));
    assert_eq!(result(4), &json!([constant]));
    assert_eq!(result(5), &json!([]));
    assert_eq!(result(6), &json!([rendered, constant]));
    assert_eq!(result(7), &json!([constant]));
    assert_eq!(result(8), result(6));
    assert_eq!(result(9), &json!([]));
}

#[test]
fn literal_inlays_survive_eager_coverage_gaps_and_respect_opaque_forms() {
    let fixture = Fixture::new("literal inlays through coverage gaps");
    let file = fixture.0.join("src/literals.clj");
    let deps = fixture.0.join("src/fixture");
    fs::create_dir(&deps).unwrap();
    fs::write(deps.join("api.clj"), "(ns fixture.api)\n(defn fetch [x] x)").unwrap();
    fs::write(
        deps.join("macros.clj"),
        "(ns fixture.macros)\n(defmacro stop [] '(throw (Exception.)))",
    )
    .unwrap();
    let source = r#"(ns literals (:require [fixture.api :as api] [fixture.macros :as macros]))
(defn ordinary [input]
  (let [fetched (api/fetch input)
        title "Done"
        rendered (str input)]
    [title rendered fetched]))
(defn opaque []
  (let [before "before"]
    (macros/stop)
    (let [after "after"] after)))
(defn evaluated-metadata []
  ^{:effect (throw (Exception.))} []
  (let [unreachable "metadata"] unreachable))
(defn hard-error []
  (subs nil 0)
  (let [invalid "error"] invalid))
(defn dead-branch []
  (when false (let [never "dead"] never)))
(defn- opaque-helper [] (macros/stop))
(defn after-opaque-helper []
  (opaque-helper)
  (let [later "opaque helper"] later))
(defn- metadata-helper [] ^{:effect (throw (Exception.))} [])
(defn after-metadata-helper []
  (let [ignored (metadata-helper)
        later "metadata helper"]
    [ignored later]))
"#;
    fs::write(&file, source).unwrap();
    let document = uri(&file);
    let replies = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"version":1,"languageId":"clojure","text":source
        }}}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/inlayHint","params":{
            "textDocument":{"uri":document},"range":{
                "start":{"line":0,"character":0},"end":{"line":source.lines().count(),"character":0}
            }
        }}),
        json!({"jsonrpc":"2.0","id":3,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let hint = |line: usize, name: &str, label: &str| {
        let row = source.lines().nth(line).unwrap();
        let end = row.find(name).unwrap() + name.len();
        json!({"position":{"line":line,"character":end},"kind":1,"label":label})
    };
    let result = &replies.iter().find(|reply| reply["id"] == 2).unwrap()["result"];
    assert_eq!(
        result,
        &json!([
            hint(3, "title", ": \"Done\""),
            hint(7, "before", ": \"before\"")
        ]),
    );
}

#[test]
fn unannotated_inlay_queries_leave_out_overgrown_conditional_types() {
    let fixture = Fixture::new("inlay conditional growth");
    let file = fixture.0.join("src/actions.clj");
    let document = uri(&file);
    let source = |choices| {
        let steps: String = (0..choices)
            .map(|index| format!(" enabled (conj :action-{index})"))
            .collect();
        format!(
            "(ns actions)\n(defn pick-actions [enabled]\n  (let [title \"Actions\"\n        actions (cond-> []{steps})]\n    [title actions]))\n",
        )
    };
    let large = source(12);
    let small = source(2);
    fs::write(&file, &small).unwrap();
    let request = |id| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/inlayHint","params":{
            "textDocument":{"uri":document},
            "range":{"start":{"line":2,"character":0},"end":{"line":3,"character":0}}
        }})
    };
    let replies = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"version":1,"languageId":"clojure","text":small
        }}}),
        request(2),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":2},"contentChanges":[{"text":large}]
        }}),
        request(3),
        json!({"jsonrpc":"2.0","id":4,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| &replies.iter().find(|reply| reply["id"] == id).unwrap()["result"];
    assert_eq!(result(2)[0]["label"], ": \"Actions\"");
    assert_eq!(result(3), &json!([]));
}

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
fn source_contract_edits_refresh_hints_in_an_unchanged_caller() {
    let fixture = Fixture::new("source contract inlay refresh");
    let caller_path = fixture.0.join("src/caller.clj");
    let provider_path = fixture.0.join("src/provider.clj");
    let caller = uri(&caller_path);
    let provider = uri(&provider_path);
    let consumer = r#"(ns caller (:require [provider :as p]))
(defn read-value {:t/sig '[:-> t/any]} []
  (let [value (p/value)] value))
"#;
    let original = r#"(ns provider)
(defn value {:t/sig '[:-> string?]} [] "value")
"#;
    let shifted = format!(";; 🦊\n{original}\n(defn- internal [] 1)\n");
    let changed = shifted
        .replace("string?", "number?")
        .replace("\"value\"", "9");
    fs::write(&caller_path, consumer).unwrap();
    fs::write(&provider_path, original).unwrap();

    let open = |uri: &str, source: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":uri,"version":1,"languageId":"clojure","text":source}
        }})
    };
    let edit = |version, text| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":provider,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let request = |id| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/inlayHint","params":{
            "textDocument":{"uri":caller},"range":{
                "start":{"line":2,"character":0},"end":{"line":3,"character":0}
            }
        }})
    };
    let replies = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&fixture.0),
            "capabilities":{"workspace":{"inlayHint":{"refreshSupport":true}}}
        }}),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        open(&caller, consumer),
        open(&provider, original),
        request(2),
        edit(2, &shifted),
        request(3),
        edit(3, &changed),
        request(4),
        json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{
            "textDocument":{"uri":provider}
        }}),
        request(5),
        json!({"jsonrpc":"2.0","id":6,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = |id| replies.iter().position(|reply| reply["id"] == id).unwrap();
    for (id, label) in [
        (2, ": String"),
        (3, ": String"),
        (4, ": Number"),
        (5, ": String"),
    ] {
        assert_eq!(
            replies[response(id)]["result"],
            json!([{"position":{"line":2,"character":13},"kind":1,"label":label}]),
        );
    }
    let refresh: Vec<_> = replies
        .iter()
        .enumerate()
        .filter(|(_, reply)| reply["method"] == "workspace/inlayHint/refresh")
        .collect();
    let [(index, request)] = refresh.as_slice() else {
        panic!("expected one in-flight refresh, got {refresh:?}");
    };
    assert!((response(3)..response(4)).contains(index));
    assert!(request["id"].is_string());
    assert!(request.get("params").is_none());
}

#[test]
fn saved_source_contracts_refresh_inlay_hints_in_unchanged_buffers() {
    let fixture = Fixture::new("closed source inlay refresh");
    let caller_path = fixture.0.join("src/caller.clj");
    let provider_path = fixture.0.join("src/provider.clj");
    let caller = uri(&caller_path);
    let provider = uri(&provider_path);
    let consumer = r#"(ns caller (:require [provider :as p]))
(defn read-value {:t/sig '[:-> t/any]} []
  (let [value (p/value)] value))
"#;
    let publish = |result, value, suffix| {
        let source =
            format!("(ns provider)\n(defn value {{:t/sig '[:-> {result}]}} [] {value})\n{suffix}");
        fs::write(&provider_path, source).unwrap();
    };
    publish("string?", "\"value\"", "");
    fs::write(&caller_path, consumer).unwrap();
    let mut child = lsp_server();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut exchange = |request: Value, notifications: &[Value]| {
        for notification in notifications {
            send_message(&mut input, notification);
        }
        send_message(&mut input, &request);
        let mut refreshes = 0;
        loop {
            let reply = read_message(&mut output).unwrap();
            if reply["id"] == request["id"] {
                assert!(reply.get("error").is_none(), "{reply}");
                return (reply["result"].clone(), refreshes);
            }
            if reply["method"] == "workspace/inlayHint/refresh" {
                assert!(
                    reply["id"].is_string() && reply.get("params").is_none(),
                    "{reply}"
                );
                refreshes += 1;
                send_message(
                    &mut input,
                    &json!({"jsonrpc":"2.0","id":reply["id"],"result":null}),
                );
            }
        }
    };
    exchange(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&fixture.0),
            "capabilities":{"workspace":{"inlayHint":{"refreshSupport":true}}}
        }}),
        &[],
    );
    let hints = |id| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/inlayHint","params":{
            "textDocument":{"uri":caller},"range":{
                "start":{"line":2,"character":0},"end":{"line":3,"character":0}
            }
        }})
    };
    let expected = |label| json!([{"position":{"line":2,"character":13},"kind":1,"label":label}]);
    let watch = json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles","params":{
        "changes":[{"uri":provider,"type":2}]
    }});
    let save = json!({"jsonrpc":"2.0","method":"textDocument/didSave","params":{
        "textDocument":{"uri":provider}
    }});
    let initial = exchange(
        hints(2),
        &[
            json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
            json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
                "uri":caller,"version":1,"languageId":"clojure","text":consumer
            }}}),
        ],
    );
    assert_eq!(initial, (expected(": String"), 0));
    publish("number?", "42", "");
    assert_eq!(
        exchange(hints(3), std::slice::from_ref(&watch)),
        (expected(": Number"), 1)
    );
    assert_eq!(
        exchange(hints(4), std::slice::from_ref(&save)),
        (expected(": Number"), 0)
    );
    publish("number?", "42", ";; Only documentation changed\n");
    assert_eq!(exchange(hints(5), &[watch]), (expected(": Number"), 0));
    publish("boolean?", "true", "");
    assert_eq!(exchange(hints(6), &[save]), (expected(": Boolean"), 1));
    exchange(json!({"jsonrpc":"2.0","id":7,"method":"shutdown"}), &[]);
    send_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
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

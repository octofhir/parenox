use super::*;

#[test]
fn alias_only_namespaces_keep_data_resolution_and_withhold_call_types() {
    let fixture = Fixture::new("alias-only call types");
    let file = fixture.0.join("src/consumer.clj");
    let document = uri(&file);
    let provider = fixture.0.join("src/provider.clj");
    fs::write(
        &provider,
        "(ns provider)\n(defn normalize \"Normalize text.\" {:t/sig '[string? :-> string?]} [text] text)\n",
    )
    .unwrap();
    let source = r#"(ns consumer (:require [provider :as-alias p]))
(defn good {:t/sig '[:-> t/any]} []
  (let [selected (p/normalize "four")] selected))
(defn bad {:t/sig '[:-> t/any]} [] (p/normalize 42))
(defn data {:t/sig '[:-> keyword?]} [] (let [key ::p/mode] key))
"#;
    fs::write(&file, source).unwrap();
    let loaded = source.replace(
        "[provider :as-alias p]",
        "[provider] [provider :as-alias p]",
    );
    let position = |line: usize, text: &str| {
        let row = source.lines().nth(line).unwrap();
        json!({"line":line,"character":row.find(text).unwrap()})
    };
    let request = |id, method, position| {
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":{
            "textDocument":{"uri":document},"position":position
        }})
    };
    let queries = |id| {
        [
            request(id, "textDocument/hover", position(2, "selected")),
            request(id + 1, "textDocument/hover", position(2, "p/normalize")),
            request(
                id + 2,
                "textDocument/signatureHelp",
                position(2, "\"four\""),
            ),
            request(
                id + 3,
                "textDocument/definition",
                position(2, "p/normalize"),
            ),
            request(id + 4, "textDocument/hover", position(4, "key ::")),
        ]
    };
    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"languageId":"clojure","version":1,"text":source
        }}}),
    ];
    requests.extend(queries(2));
    requests.push(
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":2},"contentChanges":[{"text":loaded}]
        }}),
    );
    requests.extend(queries(7));
    requests.push(json!({"jsonrpc":"2.0","id":12,"method":"shutdown"}));
    requests.push(json!({"jsonrpc":"2.0","method":"exit"}));
    let replies = transcript(&requests);
    let result = |id| &replies.iter().find(|reply| reply["id"] == id).unwrap()["result"];
    assert!(result(2).is_null(), "{}", result(2));
    assert_eq!(result(7)["contents"]["value"], "selected\nString");
    let alias_hover = result(3)["contents"]["value"].as_str().unwrap();
    assert!(alias_hover.contains("Normalize text."), "{alias_hover}");
    assert!(!alias_hover.contains("String"), "{alias_hover}");
    assert!(
        result(8)["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("String")
    );
    assert!(
        result(4)["signatures"][0]["label"]
            .as_str()
            .unwrap()
            .contains("text")
    );
    assert_eq!(
        result(9)["signatures"][0]["label"],
        "(normalize [text: String]) -> String"
    );
    assert_eq!(result(5)["uri"], uri(&provider));
    assert_eq!(result(5), result(10));
    assert_eq!(result(6)["contents"]["value"], "key\n:provider/mode");
    assert_eq!(result(6), result(11));

    let diagnostics = |version| {
        replies
            .iter()
            .find(|reply| {
                reply["method"] == "textDocument/publishDiagnostics"
                    && reply["params"]["version"] == version
            })
            .unwrap()["params"]["diagnostics"]
            .as_array()
            .unwrap()
    };
    let alias_gaps: Vec<_> = diagnostics(1)
        .iter()
        .filter(|item| item["code"] == "PNX9003")
        .collect();
    assert_eq!(alias_gaps.len(), 2, "{:?}", diagnostics(1));
    assert_eq!(alias_gaps[1]["range"]["start"], position(3, "p/normalize"));
    assert_eq!(alias_gaps[1]["range"]["end"], position(3, " 42"));
    assert!(diagnostics(1).iter().all(|item| item["code"] != "PNX2322"));
    assert!(diagnostics(2).iter().all(|item| item["code"] != "PNX9003"));
    let mismatch = diagnostics(2)
        .iter()
        .find(|item| item["code"] == "PNX2322")
        .unwrap();
    assert_eq!(mismatch["range"]["start"], position(3, "42"));
}

#[test]
fn hover_withholds_invocation_types_when_a_local_closure_escapes() {
    let fixture = Fixture::new("escaping local hover");
    let file = fixture.0.join("src/closures.clj");
    let document = uri(&file);
    let source = r#"(ns closures (:require [fixture.uri :as uri]))
(defmacro later [] '(outward {:field "later"}))
(defn inspect {:t/sig '[:-> t/any]} []
  (letfn [(inner [row]
            (let [picked (:field row)]
              (uri/forward picked)))
          (outward [row] (inner row))]
    (let [kept 42]
      (outward {:field "first"})
      (str kept)
      nil)))
"#;
    fs::write(&file, source).unwrap();
    fs::write(fixture.0.join("parenox.edn"), r#"{:externs ["uri.edn"]}"#).unwrap();
    fs::write(
        fixture.0.join("uri.edn"),
        "{fixture.uri/forward (t/nil-preserving [string? :-> string?])}",
    )
    .unwrap();

    let sites = [
        ("textDocument/hover", 3, "row"),
        ("textDocument/hover", 4, "row"),
        ("textDocument/hover", 4, ":field"),
        ("textDocument/hover", 7, "kept"),
        ("textDocument/signatureHelp", 5, "picked"),
    ];
    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"version":1,"languageId":"clojure","text":source
        }}}),
    ];
    let overlays = [
        None,
        Some("outward"),
        Some("(later)"),
        Some("(fn [input] (outward input))"),
        Some("#(outward %)"),
        Some("(if false outward nil)"),
        Some("nil"),
    ];
    for (index, overlay) in overlays.iter().enumerate() {
        if let Some(tail) = overlay {
            let changed = source.replace("nil)))", &format!("{tail})))"));
            requests.push(json!({
                "jsonrpc":"2.0","method":"textDocument/didChange","params":{
                    "textDocument":{"uri":document,"version":index+1},
                    "contentChanges":[{"text":changed}]
                }
            }));
        }
        for (site, (method, line, token)) in sites.iter().enumerate() {
            let row = source.lines().nth(*line).unwrap();
            let start = row.find(token).unwrap();
            let column = row[..start].encode_utf16().count();
            requests.push(json!({"jsonrpc":"2.0","id":2+index*sites.len()+site,
                "method":method,"params":{
                    "textDocument":{"uri":document},"position":{"line":line,"character":column}
                }
            }));
        }
    }
    requests.push(json!({"jsonrpc":"2.0","id":100,"method":"shutdown"}));
    requests.push(json!({"jsonrpc":"2.0","method":"exit"}));
    let replies = transcript(&requests);
    let result = |index, site| {
        &replies
            .iter()
            .find(|reply| reply["id"] == 2 + index * sites.len() + site)
            .unwrap()["result"]
    };
    for index in [0, 5, 6] {
        assert_eq!(
            result(index, 0)["contents"]["value"],
            result(index, 1)["contents"]["value"]
        );
        assert!(
            result(index, 0)["contents"]["value"]
                .as_str()
                .is_some_and(|contents| contents.contains("first")),
            "version {} binder: {}, lookup: {}, signature: {}",
            index + 1,
            result(index, 0),
            result(index, 2),
            result(index, 4)
        );
        assert_eq!(result(index, 2)["contents"]["value"], ":field\n\"first\"");
        assert_eq!(
            result(index, 4)["signatures"][0]["label"],
            "(fixture.uri/forward [String]) -> String"
        );
    }
    for index in 1..=4 {
        for site in 0..3 {
            assert!(result(index, site).is_null(), "{}", result(index, site));
        }
        assert_eq!(
            result(index, 4)["signatures"][0]["label"],
            "(fixture.uri/forward [Nil | String]) -> Nil | String"
        );
    }
    for index in 0..overlays.len() {
        assert_eq!(result(index, 3)["contents"]["value"], "kept\n42");
    }
}

#[test]
fn a_nil_preserving_extern_tracks_guard_edits_in_hover_and_signatures() {
    let fixture = Fixture::new("nil-preserving extern");
    let path = fixture.0.join("src/fragment.clj");
    let document = uri(&path);
    fs::write(fixture.0.join("parenox.edn"), r#"{:externs ["uri.edn"]}"#).unwrap();
    fs::write(
        fixture.0.join("uri.edn"),
        "{vendor.uri/decode (t/nil-preserving [string? :-> string?])}",
    )
    .unwrap();
    let source = r#"(ns fragment (:require [clojure.spec.alpha :as s] [vendor.uri :as uri]))
(defn encode {:t/sig '[string? :-> string?]} [text] text)
(defn normalize {:t/sig '[(s/nilable string?) :-> (s/nilable string?)]} [input]
  (when-not (nil? input)
    (let [decoded (uri/decode input)] (encode decoded))))"#;
    fs::write(&path, source).unwrap();
    let broken = source.replace("when-not", "when");
    let position = |token| {
        let row = source.lines().nth(4).unwrap();
        let byte = row.rfind(token).unwrap();
        row[..byte].encode_utf16().count()
    };
    let query = |id, method, token| {
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":{
            "textDocument":{"uri":document},"position":{"line":4,"character":position(token)}
        }})
    };
    let change = |version, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let responses = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"languageId":"clojure","version":1,"text":source
        }}}),
        query(2, "textDocument/hover", "decoded"),
        query(3, "textDocument/signatureHelp", "input"),
        change(2, &broken),
        query(4, "textDocument/hover", "decoded"),
        query(5, "textDocument/signatureHelp", "input"),
        change(3, source),
        query(6, "textDocument/hover", "decoded"),
        query(7, "textDocument/signatureHelp", "input"),
        json!({"jsonrpc":"2.0","id":8,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    for (hover, signature, ty) in [(2, 3, "String"), (4, 5, "Nil"), (6, 7, "String")] {
        let result = |id| &responses.iter().find(|reply| reply["id"] == id).unwrap()["result"];
        assert_eq!(result(hover)["contents"]["value"], format!("decoded\n{ty}"));
        assert_eq!(
            result(signature)["signatures"][0]["label"],
            format!("(vendor.uri/decode [{ty}]) -> {ty}")
        );
        assert_eq!(result(signature)["activeParameter"], 0);
    }
    for (version, count) in [(1, 0), (2, 1), (3, 0)] {
        let published = responses
            .iter()
            .find(|reply| {
                reply["method"] == "textDocument/publishDiagnostics"
                    && reply["params"]["version"] == version
            })
            .unwrap();
        let diagnostics = published["params"]["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), count, "{published}");
        if let [finding] = diagnostics.as_slice() {
            let start = position("decoded");
            assert_eq!(finding["code"], "PNX2322");
            assert_eq!(
                finding["range"],
                json!({"start":{"line":4,"character":start},"end":{"line":4,"character":start+7}})
            );
            assert!(
                finding["message"]
                    .as_str()
                    .unwrap()
                    .contains("expected String, got Nil")
            );
        }
    }
}

#[test]
fn hover_narrows_the_original_seed_in_later_some_thread_steps() {
    let fixture = Fixture::new("thread seed hover");
    let path = fixture.0.join("src/thread_get.clj");
    let document = uri(&path);
    // The last step reuses the original zloc in clojure-lsp's thread-get.
    let source = r#"(ns thread-get (:require [clojure.spec.alpha :as s]))
(defn edits {:t/sig '[string? string? :-> string?]} [original changed] changed)
(defn get-in-more {:t/sig '[(s/nilable string?) :-> (s/nilable string?)]} [zloc]
  (some->> zloc (str "🦊") str (edits zloc)))
"#;
    fs::write(&path, source).unwrap();
    let broken = source.replace("some->>", "->>");
    let column = |text: &str, read| {
        let row = text.lines().nth(3).unwrap();
        let start = if read {
            row.rfind("zloc")
        } else {
            row.find("zloc")
        }
        .unwrap();
        row[..start].encode_utf16().count()
    };
    let hover = |id, text, read| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/hover","params":{
            "textDocument":{"uri":document},"position":{"line":3,"character":column(text, read)+1}
        }})
    };
    let change = |version, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let responses = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"languageId":"clojure","version":1,"text":source
        }}}),
        hover(2, source, false),
        hover(3, source, true),
        change(2, &broken),
        hover(4, &broken, true),
        change(3, source),
        hover(5, source, true),
        json!({"jsonrpc":"2.0","id":6,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let range = |text, read| {
        let start = column(text, read);
        json!({"start":{"line":3,"character":start},"end":{"line":3,"character":start+4}})
    };
    for (id, text, read, ty) in [
        (2, source, false, "Nil | String"),
        (3, source, true, "String"),
        (4, broken.as_str(), true, "Nil | String"),
        (5, source, true, "String"),
    ] {
        let result = &responses.iter().find(|reply| reply["id"] == id).unwrap()["result"];
        assert_eq!(result["contents"]["value"], format!("zloc\n{ty}"));
        assert_eq!(result["range"], range(text, read));
    }
    for (version, count) in [(1, 0), (2, 1), (3, 0)] {
        let findings = responses
            .iter()
            .find(|reply| {
                reply["method"] == "textDocument/publishDiagnostics"
                    && reply["params"]["version"] == version
            })
            .unwrap()["params"]["diagnostics"]
            .as_array()
            .unwrap();
        assert_eq!(findings.len(), count, "{findings:?}");
        if let [diagnostic] = findings.as_slice() {
            assert_eq!(diagnostic["code"], "PNX2322");
            assert_eq!(diagnostic["range"], range(&broken, true));
        }
    }
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
fn hover_uses_the_type_at_a_local_read_after_an_unsaved_edit() {
    let fixture = Fixture::new("local hover");
    let path = fixture.0.join("src/uri_path.cljc");
    let document = uri(&path);
    // The nil-path branch follows lambdaisland.uri/join*.
    let source = r#"(ns uri-path (:require [clojure.spec.alpha :as s] [clojure.string :as str]))
(defn normalize {:t/sig '[{:path (s/nilable string?)} :-> string?]} [uri]
  (let [path (:path uri)]
    (if (nil? (:path uri))
      (str path)
      (do "🦊" (subs path 0)))))
"#;
    fs::write(&path, source).unwrap();
    let broken = source.replace("(nil? (:path uri))", "(some? (:path uri))");
    let column = |line: usize| {
        let row = source.lines().nth(line).unwrap();
        row[..row.rfind("path").unwrap()].encode_utf16().count()
    };
    let hover = |id, line| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/hover","params":{
            "textDocument":{"uri":document},
            "position":{"line":line,"character":column(line)+1}
        }})
    };
    let change = |version, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let responses = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"languageId":"clojure","version":1,"text":source
        }}}),
        hover(2, 4),
        hover(3, 5),
        change(2, &broken),
        hover(4, 4),
        hover(5, 5),
        change(3, source),
        hover(6, 5),
        json!({"jsonrpc":"2.0","id":7,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let range = |line| {
        json!({
            "start":{"line":line,"character":column(line)},
            "end":{"line":line,"character":column(line)+4}
        })
    };
    for (id, line, ty) in [
        (2, 4, "Nil"),
        (3, 5, "String"),
        (4, 4, "String"),
        (5, 5, "Nil"),
        (6, 5, "String"),
    ] {
        let result = &responses
            .iter()
            .find(|message| message["id"] == id)
            .unwrap()["result"];
        assert_eq!(result["contents"]["value"], format!("path\n{ty}"));
        assert_eq!(result["range"], range(line));
    }
    let findings = |version| {
        let message = responses
            .iter()
            .find(|message| {
                message["method"] == "textDocument/publishDiagnostics"
                    && message["params"]["version"] == version
            })
            .unwrap();
        message["params"]["diagnostics"].as_array().unwrap()
    };
    assert!(findings(1).is_empty());
    let [diagnostic] = findings(2).as_slice() else {
        panic!("unexpected diagnostics: {:?}", findings(2));
    };
    assert_eq!(diagnostic["range"], range(5));
    assert_eq!(diagnostic["code"], "PNX2322");
    assert!(
        diagnostic["message"]
            .as_str()
            .unwrap()
            .contains("expected String, got Nil")
    );
    assert!(findings(3).is_empty());
    assert_eq!(fs::read_to_string(&path).unwrap(), source);

    fs::write(&path, &broken).unwrap();
    let cli = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .current_dir(&fixture.0)
        .args(["check", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(cli.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&cli.stdout).unwrap();
    assert_eq!(report["complete"], true);
    let findings = report["diagnostics"].as_array().unwrap();
    assert_eq!(findings.len(), 1);
    let cli = &findings[0];
    assert_eq!(cli["code"], diagnostic["code"]);
    assert_eq!(cli["message"], diagnostic["message"]);
    assert_eq!(cli["expected"], "String");
    assert_eq!(cli["actual"], "Nil");
    let start = cli["span"]["start"]["offset"].as_u64().unwrap() as usize;
    assert_eq!(start, broken.rfind("path").unwrap());
}

#[test]
fn hover_follows_narrowing_inside_evaluated_vector_metadata() {
    let fixture = Fixture::new("vector metadata hover");
    let path = fixture.0.join("src/vector_hover.clj");
    let document = uri(&path);
    let source = r#"(ns vector-hover (:require [clojure.spec.alpha :as s]))
(defn render {:t/sig '[(s/nilable string?) :-> string?]} [label]
  (let [^String replacement "!"]
    (if (string? label)
      (apply str ^{:stamp (do "🦊" label)} ^label ["x" replacement])
      "fallback")))
"#;
    fs::write(&path, source).unwrap();
    let nil_branch = source.replace("(string? label)", "(nil? label)");
    let overwritten = source.replace(":stamp", ":tag");
    let stopped = source.replace("\"x\" replacement", "(subs nil 0) replacement");
    let range = |text: &str, line: usize, marker: &str, skip: usize, width: usize| {
        let row = text.lines().nth(line).unwrap();
        let start = row.find(marker).unwrap() + skip;
        let character = row[..start].encode_utf16().count();
        json!({
            "start":{"line":line,"character":character},
            "end":{"line":line,"character":character+width}
        })
    };
    let change = |version, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let mut messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"languageId":"clojure","version":1,"text":source
        }}}),
    ];
    let mut expected = Vec::new();
    for (version, text, value_type, tag_type) in [
        (1, source, Some("String"), Some("String")),
        (2, &nil_branch, Some("Nil"), Some("Nil")),
        (3, &overwritten, Some("String"), None),
        (4, &stopped, None, None),
        (5, source, Some("String"), Some("String")),
    ] {
        if version > 1 {
            messages.push(change(version, text));
        }
        for (line, marker, skip, width, ty) in [
            (4, "label)", 0, 5, value_type),
            (4, "^label", 1, 5, tag_type),
            (2, "^String", 1, 6, None),
        ] {
            let id = messages.len();
            let range = range(text, line, marker, skip, width);
            messages.push(
                json!({"jsonrpc":"2.0","id":id,"method":"textDocument/hover","params":{
                    "textDocument":{"uri":document},"position":range["start"]
                }}),
            );
            expected.push((id, range, ty));
        }
    }
    messages.push(json!({"jsonrpc":"2.0","id":messages.len(),"method":"shutdown"}));
    messages.push(json!({"jsonrpc":"2.0","method":"exit"}));
    let replies = transcript(&messages);
    for (id, range, ty) in expected {
        let result = &replies.iter().find(|reply| reply["id"] == id).unwrap()["result"];
        if let Some(ty) = ty {
            assert_eq!(result["contents"]["value"], format!("label\n{ty}"));
            assert_eq!(result["range"], range);
        } else {
            assert_eq!(result, &Value::Null);
        }
    }
}

#[test]
fn hover_requires_a_valid_call_result_and_signature_help_keeps_the_declaration() {
    let fixture = Fixture::new("rejected call hover");
    let file = fixture.0.join("src/fragment.clj");
    let document = uri(&file);
    let source = r#"(ns fragment (:require [clojure.spec.alpha :as s] [library.codec :as codec]))
(defn normalize {:t/sig '[(s/nilable string?) :-> string?]} [fragment]
  (if (nil? fragment) ""
    (let [decoded (codec/decode fragment)
          encoded (codec/encode decoded :fragment)]
      encoded)))
"#;
    let rejected = source.replace("(nil? fragment)", "(some? fragment)");
    fs::write(&file, source).unwrap();
    fs::write(
        fixture.0.join("parenox.edn"),
        r#"{:externs ["codecs.edn"]}"#,
    )
    .unwrap();
    fs::write(
        fixture.0.join("codecs.edn"),
        r#"{library.codec/decode (t/nil-preserving [string? :-> string?])
             library.codec/encode [string? keyword? :-> string?]}"#,
    )
    .unwrap();
    let request = |id, method, line: usize, token| {
        let row = source.lines().nth(line).unwrap();
        let start = row.rfind(token).unwrap();
        let character = row[..start].encode_utf16().count();
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":{
            "textDocument":{"uri":document},"position":{"line":line,"character":character}
        }})
    };
    let hover = |id, line, token| request(id, "textDocument/hover", line, token);
    let signature = |id| request(id, "textDocument/signatureHelp", 4, "decoded");
    let change = |version, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let responses = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":document,"languageId":"clojure","version":1,"text":source}
        }}),
        hover(2, 4, "encoded"),
        hover(3, 5, "encoded"),
        signature(4),
        change(2, &rejected),
        hover(5, 4, "decoded"),
        hover(6, 4, "encoded"),
        hover(7, 5, "encoded"),
        signature(8),
        change(3, source),
        hover(9, 5, "encoded"),
        signature(10),
        json!({"jsonrpc":"2.0","id":11,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| &responses.iter().find(|reply| reply["id"] == id).unwrap()["result"];
    for id in [2, 3, 9] {
        assert_eq!(result(id)["contents"]["value"], "encoded\nString");
    }
    assert_eq!(result(5)["contents"]["value"], "decoded\nNil");
    assert!(result(6).is_null());
    assert!(result(7).is_null());
    assert_eq!(result(4)["activeParameter"], 0);
    assert_eq!(result(4), result(8));
    assert_eq!(result(4), result(10));
    let diagnostics = &responses
        .iter()
        .find(|reply| {
            reply["method"] == "textDocument/publishDiagnostics" && reply["params"]["version"] == 2
        })
        .unwrap()["params"]["diagnostics"];
    let [diagnostic] = diagnostics.as_array().unwrap().as_slice() else {
        panic!("{diagnostics}");
    };
    assert_eq!(diagnostic["code"], "PNX2322");
    assert_eq!(diagnostic["range"], result(5)["range"]);
}

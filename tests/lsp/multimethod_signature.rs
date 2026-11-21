use super::*;

#[test]
fn signature_help_uses_a_literal_dispatcher_for_a_resolved_multimethod() {
    let fixture = Fixture::new("multimethod signature");
    let path = fixture.0.join("src/dispatch.clj");
    let document = uri(&path);
    let source = r#"(ns dispatch)
(defmulti render "Render a value."
  {:doc "Overridden by the positional docstring."}
  (fn dispatch
    ([format] format)
    ([format value & options] format)))
(defmulti computed identity)
(defn sample []
  (render :solo)
  (render :wide :payload :rest)
  (computed :unknown)
  (let [render identity] (render :shadow)))"#;
    fs::write(&path, source).unwrap();
    let renamed = source.replace("[format value & options]", "[format item & options]");
    let query = |id, token| {
        let (line, column) = source
            .lines()
            .enumerate()
            .find_map(|(line, text)| {
                text.find(token)
                    .map(|start| (line, text[..start].encode_utf16().count()))
            })
            .unwrap();
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/signatureHelp","params":{
            "textDocument":{"uri":document},"position":{"line":line,"character":column}
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
        query(2, ":solo"),
        query(3, ":payload"),
        query(4, ":rest"),
        query(5, ":unknown"),
        query(6, ":shadow"),
        change(2, &renamed),
        query(7, ":payload"),
        change(3, source),
        query(8, ":payload"),
        json!({"jsonrpc":"2.0","id":9,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| &responses.iter().find(|reply| reply["id"] == id).unwrap()["result"];
    for (id, arity, param, label) in [
        (2, 0, 0, "(render [format])"),
        (3, 1, 1, "(render [format value & options])"),
        (4, 1, 2, "(render [format value & options])"),
        (7, 1, 1, "(render [format item & options])"),
        (8, 1, 1, "(render [format value & options])"),
    ] {
        let result = result(id);
        assert_eq!(result["activeSignature"], arity, "{result}");
        assert_eq!(result["activeParameter"], param);
        assert_eq!(result["signatures"][arity]["label"], label);
        assert_eq!(
            result["signatures"][arity]["documentation"],
            "Render a value."
        );
    }
    assert_eq!(result(5), &Value::Null);
    assert_eq!(result(6), &Value::Null);
}

#[test]
fn signature_help_does_not_assume_core_fn_when_local_macros_reuse_its_name() {
    let fixture = Fixture::new("shadowed dispatcher");
    let path = fixture.0.join("src/dispatch.clj");
    let document = uri(&path);
    let source = r#"(ns dispatch)
(defmacro fn [& _] `(clojure.core/fn [~'a] :local))
(defmacro fn [& _] `(clojure.core/fn [~'a ~'b] :local))
(defmulti local-dispatch (fn [mistaken] :local))
(defmulti core-dispatch (clojure.core/fn [correct] :core))
(local-dispatch :left :right)
(core-dispatch :value)"#;
    fs::write(&path, source).unwrap();
    let query = |id, line, character| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/signatureHelp","params":{
            "textDocument":{"uri":document},"position":{"line":line,"character":character}
        }})
    };
    let responses = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"languageId":"clojure","version":1,"text":source
        }}}),
        query(2, 5, 16),
        query(3, 6, 15),
        json!({"jsonrpc":"2.0","id":4,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| &responses.iter().find(|reply| reply["id"] == id).unwrap()["result"];
    assert!(result(2).is_null(), "{}", result(2));
    assert_eq!(
        result(3)["signatures"][0]["label"],
        "(core-dispatch [correct])"
    );
}

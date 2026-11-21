use super::*;

#[test]
fn editor_help_uses_a_literal_dispatcher_for_a_resolved_multimethod() {
    let fixture = Fixture::new("multimethod signature");
    let path = fixture.0.join("src/dispatch.clj");
    let document = uri(&path);
    let source = r#"(ns dispatch)
(defmulti render "Render a value."
  {:doc "Overridden by the positional docstring."}
  (fn dispatch
    ([format] format)
    ([format value & options] format)))
(defmulti ^{:doc "Old docs." :arglists '([old])} computed
  {:doc "Computed dispatcher." :arglists '([item])} identity)
(defmulti ^{:doc "Old docs." :arglists '([old])} cleared
  {:doc nil :arglists nil} (fn [private-name] private-name))
(defn sample []
  (render :solo)
  (render :wide :payload :rest)
  (computed :unknown)
  (cleared :ignore)
  (let [render identity] (render :shadow)))"#;
    fs::write(&path, source).unwrap();
    let renamed = source.replace("[format value & options]", "[format item & options]");
    let query = |id, method, token| {
        let (line, column) = source
            .lines()
            .enumerate()
            .find_map(|(line, text)| {
                text.find(token)
                    .map(|start| (line, text[..start].encode_utf16().count()))
            })
            .unwrap();
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":{
            "textDocument":{"uri":document},"position":{"line":line,"character":column}
        }})
    };
    let signature = |id, token| query(id, "textDocument/signatureHelp", token);
    let hover = |id, token| query(id, "textDocument/hover", token);
    let change = |version, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let responses = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&fixture.0),"capabilities":{"textDocument":{"hover":{"contentFormat":["markdown"]}}}
        }}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"languageId":"clojure","version":1,"text":source
        }}}),
        signature(2, ":solo"),
        signature(3, ":payload"),
        signature(4, ":rest"),
        signature(5, ":unknown"),
        signature(6, ":shadow"),
        hover(9, "render :wide"),
        hover(10, "computed :unknown"),
        hover(11, "cleared :ignore"),
        change(2, &renamed),
        signature(7, ":payload"),
        hover(12, "render :wide"),
        change(3, source),
        signature(8, ":payload"),
        hover(13, "render :wide"),
        json!({"jsonrpc":"2.0","id":14,"method":"shutdown"}),
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
    let docs = |id| result(id)["contents"]["value"].as_str().unwrap();
    for (id, binding) in [(9, "value"), (12, "item"), (13, "value")] {
        let docs = docs(id);
        assert!(
            docs.starts_with(&format!(
                "```clojure\n(dispatch/render\n [format]\n [format {binding} & options])\n```"
            )),
            "{docs}"
        );
        assert!(docs.ends_with("Render a value."), "{docs}");
    }
    assert!(docs(10).starts_with("```clojure\n(dispatch/computed\n [item])\n```"));
    assert!(docs(10).ends_with("Computed dispatcher."));
    assert!(docs(11).starts_with("```clojure\ndispatch/cleared\n```"));
    assert!(!docs(11).contains("Old docs."));
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

use super::*;

#[test]
fn hover_shows_the_lookup_type_at_each_evaluated_keyword_occurrence() {
    let fixture = Fixture::new("lookup hover");
    let path = fixture.0.join("src/lookup_hover.clj");
    let document = uri(&path);
    let source = r#"(ns lookup-hover (:require [clojure.spec.alpha :as s]))
(s/def ::path string?)
(defn uri {:t/sig '[{:path (s/nilable string?)} :-> string?]} [ref]
  (if (nil? (:path ref))
    "missing"
    (do "🦊" (subs (:path ref) 0))))
(defn fallback {:t/sig '[:-> string?]} [] (get {} :path "fallback"))
(defn nested {:t/sig '[{:ref {:path string?}} :-> string?]} [base]
  (get (:ref base) :path))
(defn data {:t/sig '[:-> t/any]} [] {:path '(:path {})})
(defn shadow {:t/sig '[(s/fspec :args (s/cat :m map? :k keyword?) :ret string?) :-> string?]} [get]
  (get {} :path))
"#;
    fs::write(&path, source).unwrap();
    let broken = source.replace("(nil? (:path ref))", "(some? (:path ref))");
    let at = |text: &str, line, keyword: &str| {
        let row = text.lines().nth(line).unwrap();
        row[..row.rfind(keyword).unwrap()].encode_utf16().count()
    };
    let hover = |id, text, line, keyword| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/hover","params":{
            "textDocument":{"uri":document},"position":{"line":line,"character":at(text,line,keyword)+1}
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
        hover(2, source, 1, "::path"),
        hover(3, source, 3, ":path"),
        hover(4, source, 5, ":path"),
        hover(5, source, 6, ":path"),
        hover(6, source, 8, ":ref"),
        hover(7, source, 8, ":path"),
        hover(8, source, 9, ":path"),
        hover(9, source, 11, ":path"),
        change(2, &broken),
        hover(10, &broken, 5, ":path"),
        change(3, source),
        hover(11, source, 5, ":path"),
        json!({"jsonrpc":"2.0","id":12,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| &replies.iter().find(|reply| reply["id"] == id).unwrap()["result"];
    let spec = result(2)["contents"]["value"].as_str().unwrap();
    assert!(
        spec.contains("Inferred from s/def") && spec.contains("Input type String"),
        "{spec}"
    );
    for (id, line, key, inferred) in [
        (3, 3, ":path", "Nil | String"),
        (4, 5, ":path", "String"),
        (5, 6, ":path", "\"fallback\""),
        (6, 8, ":ref", "{:path: String}"),
        (7, 8, ":path", "String"),
        (10, 5, ":path", "Nil"),
        (11, 5, ":path", "String"),
    ] {
        let start = at(source, line, key);
        let hover = result(id);
        assert_eq!(hover["contents"]["value"], format!("{key}\n{inferred}"));
        assert_eq!(
            hover["range"],
            json!({
                "start":{"line":line,"character":start},"end":{"line":line,"character":start+key.len()}
            })
        );
    }
    assert_eq!(result(8), &Value::Null);
    assert_eq!(result(9), &Value::Null);
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
    assert!(diagnostics(1).is_empty());
    let [diagnostic] = diagnostics(2).as_slice() else {
        panic!("{replies:#?}")
    };
    assert_eq!(diagnostic["code"], "PNX2322");
    assert!(diagnostics(3).is_empty());
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

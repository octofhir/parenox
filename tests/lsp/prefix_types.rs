use super::*;

#[test]
fn a_recovered_core_namespace_cannot_enable_standard_core_spec_conversions() {
    use parenox::{Session, Source};

    let path = "src/clojure/core.clj";
    let source = r#"(ns clojure.core (:require [wire :as w]))
(defn sample {:t/sig '[::w/value :-> t/any]} [value] value)
"#;
    let externs = Source {
        path: "externs.edn",
        source: r#"{:wire/value {:ns wire :spec
          (clojure.spec.alpha/and clojure.core/integer?
            (clojure.spec.alpha/conformer clojure.core/int))}}"#,
    };
    let mut session = Session::default();
    let offset = source.rfind("value").unwrap();
    let broken = format!("{source}(defn later [] (");
    for (text, code) in [(source, "PNX9001"), (broken.as_str(), "PNX9000")] {
        let report = session.check_project(&[Source { path, source: text }], Some(externs));
        assert!(
            report.files[0]
                .diagnostics
                .iter()
                .any(|finding| finding.code == code)
        );
        assert!(report.files[1].diagnostics.is_empty(), "{report:#?}");
        assert!(session.document_hover_at(path, offset).is_none());
    }
}

#[test]
fn a_recovered_prefix_cannot_replace_a_source_spec_with_an_extern_spec() {
    use parenox::{DocumentHover, Session, Source};

    let path = "src/shadow.clj";
    let source = r#"(ns shadow (:require [clojure.spec.alpha :as s]))
(s/def ::value string?)
(defn sample {:t/sig '[::value :-> string?]} [value] (str value))
"#;
    let externs = Source {
        path: "externs.edn",
        source: r#"{:shadow/value {:ns shadow :spec
          (clojure.spec.alpha/and clojure.core/integer?
            (clojure.spec.alpha/conformer clojure.core/int))}}"#,
    };
    let mut session = Session::default();
    let offset = source.rfind("value").unwrap();
    let broken = format!("{source}(defn later [] (");
    for (text, complete) in [(source, true), (broken.as_str(), false), (source, true)] {
        let report = session.check_project(&[Source { path, source: text }], Some(externs));
        assert_eq!(report.complete, complete, "{report:#?}");
        let hover = session.document_hover_at(path, offset);
        let bindings = session.binding_types(path, 0..text.len());
        if complete {
            let Some(DocumentHover::Local(value)) = hover else {
                panic!("expected local type")
            };
            assert_eq!(value.inferred_type, "String");
            assert_eq!(bindings.len(), 1);
            assert_eq!(bindings[0].inferred_type, "String");
        } else {
            assert!(hover.is_none());
            assert!(bindings.is_empty());
        }
    }
}

#[test]
fn an_incomplete_tail_preserves_types_from_the_current_complete_prefix() {
    let fixture = Fixture::new("prefix types");
    let path = fixture.0.join("src/typed_prefix.cljc");
    let document = uri(&path);
    let source = r#"(ns typed-prefix (:require [clojure.spec.alpha :as s] [vendor.codec :as codec]))
#?(:clj
   (defn earlier {:t/sig '[(s/nilable string?) :-> t/any]} [input]
     (when-some [text input]
       (let [state #::{:prepared text}]
         (codec/parse text)))))
"#;
    let unfinished = format!("{source}(defn later [] (codec/parse ");
    let changed = unfinished.replace("nilable string?", "nilable number?");
    let unresolved = changed.replace("vendor.codec :as codec", "vendor.codec :as other");
    fs::write(&path, &unfinished).unwrap();
    fs::write(fixture.0.join("parenox.edn"), r#"{:externs ["codec.edn"]}"#).unwrap();
    fs::write(
        fixture.0.join("codec.edn"),
        "{vendor.codec/parse [[t/any :-> string?]]}",
    )
    .unwrap();

    let query = |id, method| {
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":{
            "textDocument":{"uri":document},"position":{"line":5,"character":22}
        }})
    };
    let hints = |id, start, end, end_column| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/inlayHint","params":{
            "textDocument":{"uri":document},"range":{
                "start":{"line":start,"character":0},"end":{"line":end,"character":end_column}
            }
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
            "uri":document,"languageId":"clojure","version":1,"text":unfinished
        }}}),
        query(2, "textDocument/hover"),
        query(3, "textDocument/signatureHelp"),
        hints(11, 3, 6, 20),
        hints(12, 6, 6, 20),
        hints(13, 3, 4, 0),
        change(2, &changed),
        query(4, "textDocument/hover"),
        query(5, "textDocument/signatureHelp"),
        hints(14, 3, 4, 0),
        change(3, &unresolved),
        query(6, "textDocument/hover"),
        query(7, "textDocument/signatureHelp"),
        hints(15, 3, 6, 20),
        change(4, source),
        query(8, "textDocument/hover"),
        query(9, "textDocument/signatureHelp"),
        hints(16, 3, 6, 0),
        json!({"jsonrpc":"2.0","id":10,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| &responses.iter().find(|reply| reply["id"] == id).unwrap()["result"];
    for (id, ty) in [(2, "String"), (4, "Number"), (8, "String")] {
        assert_eq!(result(id)["contents"]["value"], format!("text\n{ty}"));
        assert_eq!(
            result(id)["range"],
            json!({
                "start":{"line":5,"character":22},"end":{"line":5,"character":26}
            })
        );
    }
    for id in [3, 5, 9] {
        let signature = result(id);
        let label = signature["signatures"][0]["label"].as_str().unwrap();
        assert!(label.contains("vendor.codec/parse"), "{signature}");
        assert!(label.ends_with("String"), "{signature}");
        assert_eq!(signature["activeParameter"], 0);
    }
    assert_eq!(result(6), &Value::Null);
    assert_eq!(result(7), &Value::Null);
    let row = source.lines().nth(3).unwrap();
    let end = row.find("text").unwrap() + "text".len();
    let position = json!({"line":3,"character":row[..end].encode_utf16().count()});
    assert_eq!(
        result(13),
        &json!([{"position":position,"label":": String","kind":1}])
    );
    assert_eq!(
        result(14),
        &json!([{"position":position,"label":": Number","kind":1}])
    );
    assert_eq!(result(11), result(16));
    assert_eq!(result(12), &json!([]));
    assert_eq!(result(15), &json!([]));
    let diagnostics = |version| {
        &responses
            .iter()
            .find(|reply| {
                reply["method"] == "textDocument/publishDiagnostics"
                    && reply["params"]["version"] == version
            })
            .unwrap()["params"]["diagnostics"]
    };
    let broken = diagnostics(1).as_array().unwrap();
    assert_eq!(broken.len(), 1, "{responses:#?}");
    assert_eq!(broken[0]["code"], "PNX9000");
    assert_eq!(diagnostics(4), &json!([]));
    assert_eq!(fs::read_to_string(path).unwrap(), unfinished);
}

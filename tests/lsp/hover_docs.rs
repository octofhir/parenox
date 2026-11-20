use super::*;
use zip::{ZipWriter, write::SimpleFileOptions};

#[test]
fn hovers_source_and_dependency_docstrings() {
    let fixture = Fixture::new("hover docs");
    let provider_path = fixture.0.join("src/provider.clj");
    let provider = uri(&provider_path);
    let original = r#"(ns provider)
(defn render "Render the original value." {:t/sig '[string? :-> string?]} [value] value)
(def ^{:doc "The current setting."} setting true)
(defn- secret "Private source docs." [] nil)
"#;
    fs::write(&provider_path, original).unwrap();

    let jar = fixture.0.join("vendor.jar");
    let mut archive = ZipWriter::new(fs::File::create(&jar).unwrap());
    archive
        .start_file("vendor/tools.cljc", SimpleFileOptions::default())
        .unwrap();
    archive
        .write_all(
            br#"(ns vendor.tools)
#?(:clj (defn lookup "Look up a JVM entry." [key] key)
   :cljs (defn lookup "Look up a browser entry." [key options] options))
(defn- secret "Private archive docs." [] nil)
"#,
        )
        .unwrap();
    archive.finish().unwrap();
    let classpath = fixture.0.join("classpath.txt");
    fs::write(
        &classpath,
        std::env::join_paths([Path::new("src"), &jar])
            .unwrap()
            .as_encoded_bytes(),
    )
    .unwrap();

    let client = uri(&fixture.0.join("src/app.clj"));
    let source = r#"(ns app (:require [provider :as p] [vendor.tools :as v :refer [lookup]]))
(do "🦊" (p/render "x"))
(v/lookup :entry)
p/setting
(p/secret)
(v/secret)
(defn shadow [lookup] (lookup :entry))
"#;
    let open = |file: &str, source: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":file,"languageId":"clojure","version":1,"text":source
        }}})
    };
    let columns = |line: usize, token: &str| {
        let row = source.lines().nth(line).unwrap();
        let start = row[..row.find(token).unwrap()].encode_utf16().count();
        start..start + token.encode_utf16().count()
    };
    let hover = |id, line, token| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/hover","params":{
            "textDocument":{"uri":client},"position":{"line":line,"character":columns(line, token).start + 2}
        }})
    };
    let updated = original.replace("Render the original value.", "Render the edited value.");
    let messages = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&fixture.0),
            "capabilities":{"textDocument":{"hover":{"contentFormat":["markdown","plaintext"]}}}
        }}),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        open(&client, source),
        hover(2, 1, "p/render"),
        hover(3, 2, "v/lookup"),
        hover(4, 3, "p/setting"),
        hover(5, 4, "p/secret"),
        hover(6, 5, "v/secret"),
        hover(7, 6, "lookup :entry"),
        open(&provider, original),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":provider,"version":2},"contentChanges":[{"text":updated}]
        }}),
        hover(8, 1, "p/render"),
        json!({"jsonrpc":"2.0","id":9,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ];
    let server = lsp_server_args(
        Path::new(env!("CARGO_BIN_EXE_parenox")),
        ["lsp", "--classpath", classpath.to_str().unwrap()],
    );
    let responses = transcript_with(server, &messages);
    let response = |id| {
        &responses
            .iter()
            .find(|message| message["id"] == id)
            .unwrap()["result"]
    };
    let markdown = |id| {
        let contents = &response(id)["contents"];
        assert_eq!(contents["kind"], "markdown", "{contents}");
        contents["value"].as_str().unwrap()
    };
    let text = markdown(2);
    assert!(
        text.contains("```clojure\n(provider/render\n [value])\n```"),
        "{text}"
    );
    assert!(text.contains("Render the original value."), "{text}");
    assert!(text.contains("[String -> String]"), "{text}");
    assert!(text.contains(":t/sig"), "{text}");
    assert_eq!(
        response(2)["range"],
        json!({"start":{"line":1,"character":10},"end":{"line":1,"character":18}})
    );

    let text = markdown(3);
    assert!(
        text.contains("```clojure\n(vendor.tools/lookup\n [key])\n```"),
        "{text}"
    );
    assert!(text.contains("Look up a JVM entry."), "{text}");
    assert!(markdown(4).contains("The current setting."));
    for id in [5, 6, 7] {
        assert!(response(id).is_null(), "hover {id}: {}", response(id));
    }

    let text = markdown(8);
    assert!(text.contains("Render the edited value."), "{text}");
    assert!(!text.contains("Render the original value."), "{text}");
    assert_eq!(fs::read_to_string(provider_path).unwrap(), original);
}

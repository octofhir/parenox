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

const CODE_LENS_SPEC: &str = ":server-capabilities/code-lens-provider";
const CODE_LENS_SOURCE: &str = r#"(ns wire (:require [clojure.spec.alpha :as s]))
(s/def :server-capabilities/code-lens-provider
  (s/conformer (fn [element] {:resolve-provider element})))
"#;

#[test]
fn hovers_inferred_spec_at_its_declaration_and_checked_uses() {
    let fixture = Fixture::new("hover spec");
    let provider_path = fixture.0.join("src/wire.clj");
    let client_path = fixture.0.join("src/app.clj");
    let source = r#"(ns app (:require [server-capabilities :as-alias caps] [clojure.spec.alpha :as s]))
(defn valid-wrapper? [value] (do "🦊" (s/valid? ::caps/code-lens-provider value)))
(defn data [] ::caps/code-lens-provider)
"#;
    fs::write(&provider_path, CODE_LENS_SOURCE).unwrap();
    fs::write(&client_path, source).unwrap();

    let open = |path: &Path, source: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":uri(path),"languageId":"clojure","version":1,"text":source
        }}})
    };
    let (declaration, declaration_range) =
        spec_hover_params(&provider_path, CODE_LENS_SOURCE, 1, CODE_LENS_SPEC);
    let (usage, usage_range) =
        spec_hover_params(&client_path, source, 1, "::caps/code-lens-provider");
    let (data, _) = spec_hover_params(&client_path, source, 2, "::caps/code-lens-provider");
    let hover = |id, params: Value| json!({"jsonrpc":"2.0","id":id,"method":"textDocument/hover","params":params});
    let edited = format!(
        "(ns wire (:require [clojure.spec.alpha :as s]))\n\n(s/def {CODE_LENS_SPEC} string?)\n"
    );
    let replies = transcript(&[
        spec_hover_initialize(&fixture.0),
        open(&client_path, source),
        hover(3, usage.clone()),
        open(&provider_path, CODE_LENS_SOURCE),
        hover(2, declaration),
        hover(4, data),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri(&provider_path),"version":2},"contentChanges":[{"text":edited}]
        }}),
        hover(5, usage),
        json!({"jsonrpc":"2.0","id":6,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    for (id, range) in [(2, declaration_range), (3, usage_range.clone())] {
        let result = spec_hover_result(&replies, id);
        assert_eq!(result["range"], range);
        assert_eq!(result["contents"]["kind"], "markdown");
        let text = result["contents"]["value"].as_str().unwrap();
        assert!(text.contains(CODE_LENS_SPEC), "{text}");
        assert!(text.contains("Input type Input (Any)"), "{text}");
        assert!(
            text.contains("Result type {:resolve-provider: Input}"),
            "{text}"
        );
        assert!(text.contains("Inferred from s/def"), "{text}");
        assert!(text.contains("wire.clj at line 2"), "{text}");
    }
    assert!(spec_hover_result(&replies, 4).is_null());

    let updated = spec_hover_result(&replies, 5);
    assert_eq!(updated["range"], usage_range);
    let text = updated["contents"]["value"].as_str().unwrap();
    assert!(text.contains("Input type String"), "{text}");
    assert!(
        text.contains("Result type String | :clojure.spec.alpha/invalid"),
        "{text}"
    );
    assert!(text.contains("wire.clj at line 3"), "{text}");
    assert_eq!(fs::read_to_string(provider_path).unwrap(), CODE_LENS_SOURCE);
}

#[test]
#[ignore = "requires the pinned Clojure LSP executable"]
fn compares_spec_declaration_hover_with_reference() {
    let fixture = Fixture::new("spec hover comparison");
    let root = fixture.0.canonicalize().unwrap();
    let path = root.join("src/wire.clj");
    fs::write(&path, CODE_LENS_SOURCE).unwrap();

    let (params, range) = spec_hover_params(&path, CODE_LENS_SOURCE, 1, CODE_LENS_SPEC);
    let probe = |mut server: Child| {
        let mut input = server.stdin.take().unwrap();
        let mut output = BufReader::new(server.stdout.take().unwrap());
        let mut exchange = |input: &mut std::process::ChildStdin, message: Value| {
            let id = message["id"].clone();
            send_message(input, &message);
            loop {
                let reply = read_message(&mut output).expect("LSP response");
                if reply["id"] == id && reply.get("method").is_none() {
                    assert!(reply["error"].is_null(), "{reply}");
                    return reply["result"].clone();
                }
                if reply.get("id").is_some() && reply.get("method").is_some() {
                    send_message(
                        input,
                        &json!({"jsonrpc":"2.0","id":reply["id"],"result":null}),
                    );
                }
            }
        };
        exchange(&mut input, spec_hover_initialize(&root));
        send_message(
            &mut input,
            &json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        );
        send_message(
            &mut input,
            &json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
                "uri":uri(&path),"languageId":"clojure","version":1,"text":CODE_LENS_SOURCE
            }}}),
        );
        let result = exchange(
            &mut input,
            json!({"jsonrpc":"2.0","id":2,"method":"textDocument/hover","params":params}),
        );
        exchange(
            &mut input,
            json!({"jsonrpc":"2.0","id":3,"method":"shutdown"}),
        );
        send_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"}));
        drop(input);
        assert!(server.wait().unwrap().success());
        result
    };
    let reference = Command::new(
        std::env::var_os("PARENOX_REFERENCE_LSP").unwrap_or_else(|| "clojure-lsp".into()),
    )
    .arg("listen")
    .current_dir(&root)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .spawn()
    .unwrap();
    let reference = probe(reference);
    let candidate = probe(lsp_server());
    for (server, result) in [("Clojure LSP", &reference), ("Parenox", &candidate)] {
        assert_eq!(result["range"], range, "{server} responded {result}");
        assert_eq!(result["contents"]["kind"], "markdown", "{result}");
        let text = result["contents"]["value"].as_str().unwrap();
        assert!(
            text.contains(CODE_LENS_SPEC.trim_start_matches(':')),
            "{text}"
        );
        assert!(text.contains("wire.clj"), "{text}");
    }
    let reference = reference["contents"]["value"].as_str().unwrap();
    assert!(!reference.contains("Input type"), "{reference}");
    let candidate = candidate["contents"]["value"].as_str().unwrap();
    assert!(candidate.contains("Input type Input (Any)"), "{candidate}");
    assert!(
        candidate.contains("Result type {:resolve-provider: Input}"),
        "{candidate}"
    );
}

fn spec_hover_initialize(root: &Path) -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "rootUri":uri(root),
        "capabilities":{"textDocument":{"hover":{"contentFormat":["markdown","plaintext"]}}}
    }})
}

fn spec_hover_params(path: &Path, source: &str, line: usize, keyword: &str) -> (Value, Value) {
    let row = source.lines().nth(line).unwrap();
    let start = row[..row.find(keyword).unwrap()].encode_utf16().count();
    let end = start + keyword.encode_utf16().count();
    (
        json!({"textDocument":{"uri":uri(path)},"position":{"line":line,"character":start + 2}}),
        json!({"start":{"line":line,"character":start},"end":{"line":line,"character":end}}),
    )
}

fn spec_hover_result(replies: &[Value], id: usize) -> &Value {
    &replies.iter().find(|reply| reply["id"] == id).unwrap()["result"]
}

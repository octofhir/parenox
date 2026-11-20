use super::*;

const SNAPSHOT: &str = include_str!("../fixtures/clojuredocs.edn");

fn raw(id: usize, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"clojure/clojuredocs/raw","params":params})
}

#[test]
fn clojuredocs_raw_and_hover_use_a_local_compact_export() {
    let fixture = Fixture::new("clojuredocs export");
    let export = fixture.0.join("compact docs.edn");
    fs::write(&export, SNAPSHOT).unwrap();
    let path = fixture.0.join("src/demo.clj");
    let file = uri(&path);
    let saved = "(ns demo)\n(defn f [xs] (map inc xs))\n(defn local [map] (map []))\n";
    let unsaved = "(ns demo)\n(defn f [xs] (/ 8 2))\n";
    fs::write(&path, saved).unwrap();

    for format in ["markdown", "plaintext"] {
        let server = lsp_server_args(
            Path::new(env!("CARGO_BIN_EXE_parenox")),
            ["lsp", "--clojuredocs", export.to_str().unwrap()],
        );
        let hover = |id, line, character| {
            json!({"jsonrpc":"2.0","id":id,"method":"textDocument/hover","params":{
                "textDocument":{"uri":file},"position":{"line":line,"character":character}
            }})
        };
        let messages = transcript_with(
            server,
            &[
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                    "rootUri":uri(&fixture.0),"capabilities":{"textDocument":{"hover":{"contentFormat":[format]}}}
                }}),
                raw(2, json!({"symName":"map","symNs":"clojure.core"})),
                raw(3, json!({"sym-name":"/","sym-ns":"clojure.core"})),
                raw(4, json!({"symName":"absent","symNs":"clojure.core"})),
                raw(5, json!(["map", "clojure.core"])),
                json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
                    "textDocument":{"uri":file,"version":1,"languageId":"clojure","text":saved}
                }}),
                hover(6, 1, 15),
                hover(7, 2, 20),
                json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
                    "textDocument":{"uri":file,"version":2},"contentChanges":[{"text":unsaved}]
                }}),
                hover(8, 1, 14),
                json!({"jsonrpc":"2.0","id":9,"method":"shutdown"}),
                json!({"jsonrpc":"2.0","method":"exit"}),
            ],
        );
        let result = |id| {
            let response = messages.iter().find(|message| message["id"] == id).unwrap();
            assert!(response.get("error").is_none(), "{response}");
            &response["result"]
        };
        assert_eq!(
            result(1)["capabilities"]["experimental"]["clojuredocs"],
            true
        );
        assert_eq!(
            result(2),
            &json!({
                "ns":"clojure.core","name":"map","type":"var","doc":"Map fixture documentation.",
                "arglists":["[f coll]"],"examples":["(map inc [1 2])\n;; (2 3)"],
                "seeAlsos":["clojure.core/mapv"],"notes":["Fixture note with Unicode 🧪."],
                "libraryUrl":"https://clojure.org","line":12,"static":true
            })
        );
        assert_eq!(result(3)["name"], "/");
        assert!(result(3).get("notes").unwrap().is_null());
        assert!(result(4).is_null());
        assert!(result(5).is_null());
        assert_eq!(
            result(6)["range"],
            json!({
                "start":{"line":1,"character":14},"end":{"line":1,"character":17}
            })
        );
        assert_eq!(result(6)["contents"]["kind"], format);
        let contents = result(6)["contents"]["value"].as_str().unwrap();
        for fragment in [
            "clojure.core/map",
            "Examples",
            "(map inc [1 2])",
            "clojure.core/mapv",
            "Unicode 🧪",
        ] {
            assert!(contents.contains(fragment), "{contents}");
        }
        assert_eq!(contents.contains("```clojure"), format == "markdown");
        assert!(
            result(7).is_null(),
            "local binding should shadow clojure.core/map"
        );
        let updated = result(8)["contents"]["value"].as_str().unwrap();
        assert!(updated.contains("(/ 8 2)"), "{updated}");
    }
    assert_eq!(fs::read_to_string(path).unwrap(), saved);
}

#[test]
fn clojuredocs_without_an_export_returns_null() {
    let fixture = Fixture::new("no clojuredocs export");
    let messages = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        raw(2, json!({"symName":"map","symNs":"clojure.core"})),
        json!({"jsonrpc":"2.0","id":3,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let response = messages.iter().find(|message| message["id"] == 2).unwrap();
    assert_eq!(response, &json!({"jsonrpc":"2.0","id":2,"result":null}));
}

#[test]
fn clojuredocs_reports_invalid_edn_with_its_path() {
    let fixture = Fixture::new("invalid clojuredocs");
    let export = fixture.0.join("invalid-docs.edn");
    fs::write(&export, "{:clojure.core/map {:doc \"broken").unwrap();
    let output = lsp_server_args(
        Path::new(env!("CARGO_BIN_EXE_parenox")),
        ["lsp", "--clojuredocs", export.to_str().unwrap()],
    )
    .wait_with_output()
    .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("invalid-docs.edn"), "{error}");
    assert!(error.contains("Unterminated string"), "{error}");
}

#[test]
fn clojuredocs_offline_reads_the_cache_with_an_empty_path() {
    let fixture = Fixture::new("offline clojuredocs");
    let cache = fixture.0.join("cache/parenox");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("clojuredocs.edn"), SNAPSHOT).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args(["lsp", "--clojuredocs-offline"])
        .env("XDG_CACHE_HOME", fixture.0.join("cache"))
        .env("PATH", fixture.0.join("empty-path"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let messages = transcript_with(
        child,
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
            raw(2, json!({"symName":"map","symNs":"clojure.core"})),
            json!({"jsonrpc":"2.0","id":3,"method":"shutdown"}),
            json!({"jsonrpc":"2.0","method":"exit"}),
        ],
    );
    assert_eq!(messages[1]["result"]["doc"], "Map fixture documentation.");
    assert!(!cache.join("clojuredocs.lock").exists());
}

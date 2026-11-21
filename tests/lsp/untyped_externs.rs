use super::*;

const SOURCE: &str = r#"(ns malli.edn
  (:require [edamame.core :as edamame :refer [parse-string]]
            [fixture.macros :as macros]
            [other.reader :as-alias unloaded]))

(defn parse [x options]
  (edamame/parse-string x (or options {:regex true})))
(defn local [parse-string options] (parse-string 0 options))
(defn skipped [options] (macros/ignore (edamame/parse-string 0 options)))
(defn unknown [options] (unloaded/parse 0 options))
"#;

const EXTERNS: &str = r#"{edamame.core/parse-string [[string? (clojure.spec.alpha/nilable map?) :-> t/any]]
                         other.reader/parse [[string? t/any :-> t/any]]
                         fixture.macros/ignore [[t/any :-> t/any]]}"#;

#[test]
fn external_calls_in_untyped_bodies_recheck_overlays_and_extern_edits() {
    let fixture = Fixture::new("untyped externs");
    let path = fixture.0.join("src/edn.cljc");
    let macros = fixture.0.join("src/macros.clj");
    let externs = fixture.0.join("externs.edn");
    let broken = SOURCE.replace("parse-string x ", "parse-string 0 ");
    fs::write(&path, &broken).unwrap();
    fs::write(&macros, "(ns fixture.macros)\n(defmacro ignore [_] nil)").unwrap();
    fs::write(&externs, EXTERNS).unwrap();
    fs::write(
        fixture.0.join("parenox.edn"),
        r#"{:externs ["externs.edn"]}"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .current_dir(&fixture.0)
        .args(["check", "--allow-incomplete", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let cli: Vec<_> = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|file| file["diagnostics"].as_array().unwrap())
        .filter(|finding| finding["code"] == "PNX2322")
        .collect();
    let [cli] = cli.as_slice() else {
        panic!("{report}")
    };
    assert_eq!(report["complete"], false);
    assert_eq!(cli["expected"], "String");
    assert_eq!(cli["actual"], "Number");
    assert_eq!(cli["contract"]["name"], "edamame.core/parse-string");
    let start = cli["span"]["start"]["offset"].as_u64().unwrap() as usize;
    let end = cli["span"]["end"]["offset"].as_u64().unwrap() as usize;
    assert_eq!(&broken[start..end], "0");
    fs::write(&path, SOURCE).unwrap();

    let mut child = lsp_server();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut exchange = |message| {
        send_message(&mut input, &message);
        read_message(&mut output).unwrap()
    };
    assert_eq!(
        exchange(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "rootUri":uri(&fixture.0),"capabilities":{"workspace":{"didChangeWatchedFiles":{"dynamicRegistration":true}}}
            }})
        )["id"],
        1
    );
    let register = exchange(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
    assert_eq!(register["method"], "client/registerCapability");
    send_message(
        &mut input,
        &json!({"jsonrpc":"2.0","id":register["id"],"result":null}),
    );
    let mut exchange = |message| {
        send_message(&mut input, &message);
        read_message(&mut output).unwrap()
    };
    let open = exchange(
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":uri(&path),"languageId":"clojure","version":1,"text":SOURCE}
        }}),
    );
    let edit = exchange(
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri(&path),"version":2},"contentChanges":[{"text":broken}]
        }}),
    );
    let watched = || {
        json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles","params":{
            "changes":[{"uri":uri(&externs),"type":2}]
        }})
    };
    fs::write(&externs, "{}").unwrap();
    let removed = exchange(watched());
    fs::write(&externs, EXTERNS).unwrap();
    let restored = exchange(watched());
    assert_eq!(
        exchange(json!({"jsonrpc":"2.0","id":2,"method":"shutdown"}))["id"],
        2
    );
    send_message(&mut input, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());

    for (publication, version, errors) in [
        (open, 1, 0),
        (edit, 2, 1),
        (removed, 2, 0),
        (restored, 2, 1),
    ] {
        assert_eq!(publication["method"], "textDocument/publishDiagnostics");
        assert_eq!(publication["params"]["version"], version);
        let diagnostics = publication["params"]["diagnostics"].as_array().unwrap();
        assert!(
            diagnostics
                .iter()
                .any(|finding| finding["code"] == "PNX9002")
        );
        let invalid: Vec<_> = diagnostics
            .iter()
            .filter(|finding| finding["code"] == "PNX2322")
            .collect();
        assert_eq!(invalid.len(), errors, "{publication}");
        if let [diagnostic] = invalid.as_slice() {
            assert!(diagnostic.get("relatedInformation").is_none());
            assert_eq!(
                diagnostic["range"],
                json!({
                    "start":{"line":6,"character":24},"end":{"line":6,"character":25}
                })
            );
            let message = diagnostic["message"].as_str().unwrap();
            assert_eq!(message.lines().next().unwrap(), cli["message"]);
            assert!(message.contains("edamame.core/parse-string"));
            assert!(message.contains("externs.edn"));
        }
    }
    assert_eq!(fs::read_to_string(path).unwrap(), SOURCE);
}

#[test]
fn extern_locations_follow_the_saved_contract_and_hide_in_dirty_buffers() {
    let fixture = Fixture::new("related externs");
    let path = fixture.0.join("src/app.clj");
    let externs = fixture.0.join("externs.edn");
    let source = r#"(ns app (:require [vendor.words :as w]))
(defn parse {:t/sig '[:-> string?]} [] (do "🦊" (w/read-word 42)))"#;
    let saved = "{#_\"😀\" vendor.words/read-word [string? :-> string?]}\n";
    fs::write(&path, source).unwrap();
    fs::write(&externs, saved).unwrap();
    fs::write(
        fixture.0.join("parenox.edn"),
        r#"{:externs ["externs.edn"]}"#,
    )
    .unwrap();
    #[cfg(unix)]
    let edited = {
        let alias = fixture.0.join("extern alias.edn");
        std::os::unix::fs::symlink(&externs, &alias).unwrap();
        alias
    };
    #[cfg(not(unix))]
    let edited = externs.clone();
    let document = uri(&path);
    let edn = uri(&edited);
    let change = |version, text| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":edn,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let responses = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&fixture.0),"capabilities":{"textDocument":{"publishDiagnostics":{"relatedInformation":true}}}
        }}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":document,"languageId":"clojure","version":1,"text":source}
        }}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":edn,"languageId":"edn","version":1,"text":saved}
        }}),
        change(2, "{}"),
        change(3, "\n{}"),
        json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":edn}}}),
        json!({"jsonrpc":"2.0","id":2,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let errors: Vec<_> = responses
        .iter()
        .filter(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == document
        })
        .flat_map(|message| {
            assert_eq!(message["params"]["version"], 1);
            message["params"]["diagnostics"].as_array().unwrap()
        })
        .filter(|finding| finding["code"] == "PNX2322")
        .collect();
    let [initial, dirty, restored] = errors.as_slice() else {
        panic!("{responses:?}");
    };
    assert_eq!(initial, restored);
    let related = initial["relatedInformation"].as_array().unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(
        related[0]["location"],
        json!({"uri":uri(&externs),"range":{
            "start":{"line":0,"character":8},"end":{"line":0,"character":30}
        }})
    );
    assert!(
        related[0]["message"]
            .as_str()
            .unwrap()
            .contains("vendor.words/read-word")
    );
    let mut without_location = (*initial).clone();
    without_location
        .as_object_mut()
        .unwrap()
        .remove("relatedInformation");
    assert_eq!(&without_location, *dirty);
    assert_eq!(fs::read_to_string(externs).unwrap(), saved);
}

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

struct Client {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Client {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_parenox"))
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        Self {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
        }
    }

    fn send(&mut self, request: Value) -> Value {
        writeln!(self.input, "{request}").unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], request["id"], "{response}");
        response
    }

    fn result(&mut self, request: Value) -> Value {
        let mut response = self.send(request);
        assert!(response["result"].is_object(), "{response}");
        response["result"].take()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const CONSUMER: &str = r#"(ns app (:require [store :as store]))
    (defn title {:t/sig '[string? :-> string?]} [name]
      (db/title (store/normalize name)))"#;
const PROVIDER: &str = r#"(ns store)
    (defn normalize {:t/sig '[string? :-> string?]} [name] (str name))"#;
const EXTERNS: &str = "{db/title [string? :-> string?]}";

#[test]
fn tracks_edits_and_drops_removed_providers_and_externs() {
    let mut client = Client::start();
    let opened = client.result(json!({
        "op": "open", "id": 1,
        "files": [
            {"path": "store.clj", "source": PROVIDER},
            {"path": "app.clj", "source": CONSUMER}
        ],
        "externs": {"path": "externs.edn", "source": EXTERNS}
    }));
    assert_eq!(opened["ok"], true, "{opened}");
    let warm = client.result(json!({"op": "update", "id": 2}));
    assert_eq!(warm["stats"]["parsed_files"], 0, "{warm}");
    assert_eq!(warm["stats"]["reused_functions"], 2, "{warm}");
    assert_eq!(warm["files"][0]["path"], "app.clj");

    let edited = client.result(json!({
        "op": "update", "id": 3, "changes": [{
            "path": "store.clj",
            "source": PROVIDER.replace("(str name)", "(str \"ID: \" name)")
        }]
    }));
    assert_eq!(edited["ok"], true, "{edited}");
    assert_eq!(edited["stats"]["parsed_files"], 1, "{edited}");
    assert_eq!(edited["stats"]["checked_functions"], 1, "{edited}");

    let removed = client.result(json!({"op": "update", "id": 4, "remove": ["store.clj"]}));
    assert_eq!(removed["files"].as_array().unwrap().len(), 2, "{removed}");
    assert_eq!(removed["files"][0]["diagnostics"][0]["code"], "PNX9003");

    let missing_external = client.result(json!({
        "op": "update", "id": 5,
        "changes": [{"path": "store.clj", "source": PROVIDER}], "externs": null
    }));
    assert_eq!(missing_external["files"].as_array().unwrap().len(), 2);
    assert_eq!(
        missing_external["files"][0]["diagnostics"][0]["code"],
        "PNX9003"
    );
    assert!(
        missing_external["files"][0]["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("db/title")
    );

    let restored = client.result(json!({
        "op": "update", "id": 6, "externs": {"path": "externs.edn", "source": EXTERNS}
    }));
    assert_eq!(restored["ok"], true, "{restored}");
    assert_eq!(restored["files"].as_array().unwrap().len(), 3);
}

#[test]
fn request_errors_are_atomic_and_legacy_snapshots_are_independent() {
    let mut client = Client::start();
    let unopened = client.send(json!({"op": "update", "id": "premature"}));
    assert_eq!(unopened["error"]["code"], "invalid_request");
    let opened = client.result(json!({
        "op": "open", "files": [{"path": "first.clj", "source": PROVIDER}]
    }));
    assert_eq!(opened["ok"], true);

    for request in [
        json!({"op": "update", "id": 1,
               "changes": [{"path": "first.clj", "source": "broken"}],
               "remove": ["first.clj"]}),
        json!({"op": "update", "id": 2,
               "changes": [{"path": "second.clj", "source": ""}],
               "externs": {"path": "first.clj", "source": "{}"}}),
        json!({"op": "open", "id": 3, "files": [
                {"path": "second.clj", "source": ""}, {"path": "second.clj", "source": ""}]}),
    ] {
        assert_eq!(client.send(request)["error"]["code"], "invalid_request");
    }

    let one_off = client.result(json!({
        "id": 4, "files": [{"path": "first.clj", "source": "(ns another)"}]
    }));
    assert_eq!(one_off["ok"], true);
    assert_ne!(
        one_off["files"][0]["source_hash"],
        opened["files"][0]["source_hash"]
    );

    let retained = client.result(json!({"op": "update", "id": 5}));
    assert_eq!(retained["files"].as_array().unwrap().len(), 1, "{retained}");
    assert_eq!(
        retained["files"][0]["source_hash"],
        opened["files"][0]["source_hash"]
    );

    let replaced = client.result(json!({"op": "open", "id": 6, "files": []}));
    assert_eq!(replaced["files"], json!([]));
}

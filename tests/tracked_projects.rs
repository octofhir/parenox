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
fn closing_one_diagnostic_keeps_the_other_occurrence_id() {
    let mut client = Client::start();
    let source = r#"(ns app)
        (defn first-label {:t/sig '[:-> string?]} [] 42)
        (defn second-label {:t/sig '[:-> string?]} [] 23)"#;
    let opened = client.result(json!({
        "id": 1, "op": "open", "files": [{"path":"app.clj","source":source}]
    }));
    let diagnostics = opened["files"][0]["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0]["code"], diagnostics[1]["code"]);
    assert_ne!(diagnostics[0]["id"], diagnostics[1]["id"]);
    assert!(diagnostics[1]["id"].is_string());

    let warm = client.result(json!({"id": 2, "op": "update"}));
    assert_eq!(
        warm["files"][0]["diagnostics"].as_array().unwrap(),
        diagnostics
    );
    let fixed = client.result(json!({
        "id": 3, "op": "update",
        "changes": [{"path":"app.clj","source":source.replace("[] 42", "[] \"\"")}]
    }));
    assert_eq!(fixed["files"][0]["stats"]["reused_functions"], 1);
    let remaining = fixed["files"][0]["diagnostics"].as_array().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0]["id"], diagnostics[1]["id"]);
}

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

#[test]
fn replacing_an_extern_in_the_sources_own_namespace_invalidates_warm_checks() {
    let mut client = Client::start();
    let source = r#"(ns runtime)
        (defn run {:t/sig '[:-> number?]} [] (inc (backend)))
        (defn run-qualified {:t/sig '[:-> number?]} [] (inc (runtime/backend)))"#;
    let opened = client.result(json!({
        "id": 1, "op": "open", "files": [{"path": "runtime.clj", "source": source}],
        "externs": {"path": "external.edn", "source": "{runtime/backend [:-> number?]}"}
    }));
    assert_eq!(opened["ok"], true, "{opened}");
    let warm = client.result(json!({"id": 2, "op": "update"}));
    assert_eq!(warm["stats"]["reused_functions"], 2, "{warm}");

    let changed = client.result(json!({
        "id": 3, "op": "update",
        "externs": {"path": "external.edn", "source": "{runtime/backend [:-> string?]}"}
    }));
    assert_eq!(changed["ok"], false, "{changed}");
    assert_eq!(changed["stats"]["checked_functions"], 2, "{changed}");
    assert_eq!(changed["stats"]["parsed_files"], 1, "{changed}");
    for d in changed["files"][0]["diagnostics"].as_array().unwrap() {
        assert_eq!(d["code"], "PNX2322");
        assert_eq!(d["actual"], "String");
    }

    let removed = client.result(json!({"id": 4, "op": "update", "externs": null}));
    assert_eq!(removed["stats"]["checked_functions"], 2, "{removed}");
    let diagnostics = removed["files"][0]["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 2, "{removed}");
    assert!(diagnostics.iter().all(|d| d["code"] == "PNX2304"));

    let restored = client.result(json!({
        "id": 5, "op": "update",
        "externs": {"path": "external.edn", "source": "{runtime/backend [:-> number?]}"}
    }));
    assert_eq!(restored["ok"], true, "{restored}");
}

#[test]
fn tracked_projects_isolate_cache_entries_and_release_removed_snapshots() {
    let mut client = Client::start();
    let file = json!({"path": "store.clj", "source": PROVIDER});
    let externs = |path| json!({"path": path, "source": EXTERNS});
    let opened = client.result(json!({
        "op": "open", "files": [file], "externs": externs("first.edn")
    }));
    assert_eq!(opened["stats"]["parsed_files"], 2);

    let one_off = client.result(json!({"path": "store.clj", "source": "(ns temporary)"}));
    assert_eq!(one_off["ok"], true);
    let unchanged = client.result(json!({"op": "update"}));
    assert_eq!(unchanged["stats"]["parsed_files"], 0, "{unchanged}");
    assert_eq!(unchanged["stats"]["reused_functions"], 1, "{unchanged}");

    let removed = client.result(json!({
        "op": "update", "remove": ["store.clj"], "externs": externs("next.edn")
    }));
    assert_eq!(removed["ok"], true);
    let restored = client.result(json!({
        "op": "update", "changes": [file], "externs": externs("first.edn")
    }));
    assert_eq!(restored["stats"]["parsed_files"], 2, "{restored}");
    assert_eq!(restored["stats"]["checked_functions"], 1, "{restored}");

    let reopened = client.result(json!({"op": "open", "files": [file]}));
    assert_eq!(reopened["stats"]["parsed_files"], 1, "{reopened}");
    assert_eq!(reopened["stats"]["checked_functions"], 1, "{reopened}");
}

#[test]
fn repeated_file_edits_rebase_reused_checks_and_drop_deleted_results() {
    let mut client = Client::start();
    let original = "(ns shifted)\n(defn unchanged {:t/sig '[:-> number?]} [] 7)\n(defn removed {:t/sig '[:-> number?]} [] \"invalid\")";
    let update = |client: &mut Client, id, source| {
        client.result(json!({
            "id": id,
            "op": "update",
            "changes": [{"path": "shifted.clj", "source": source}]
        }))
    };
    let opened = client.result(json!({
        "id": 1,
        "op": "open",
        "files": [{"path": "shifted.clj", "source": original}]
    }));
    assert_eq!(opened["stats"]["checked_functions"], 2);
    assert_eq!(
        opened["files"][0]["diagnostics"][0]["span"]["start"]["line"],
        3
    );

    let shifted = update(&mut client, 2, format!("; first edit\n{original}"));
    assert_eq!(shifted["stats"]["reused_functions"], 1);
    assert_eq!(
        shifted["files"][0]["diagnostics"][0]["span"]["start"]["line"],
        4
    );

    let broken = update(&mut client, 3, format!("{original}\n("));
    assert_eq!(broken["stats"]["checked_functions"], 0);
    assert_eq!(broken["files"][0]["diagnostics"][0]["code"], "PNX9000");

    let without_error = original.lines().take(2).collect::<Vec<_>>().join("\n");
    let replacement = format!(
        "; restored in a second location\n\n{without_error}\n(defn new {{:t/sig '[:-> number?]}} [] 9)"
    );
    let restored = update(&mut client, 4, replacement);
    assert_eq!(restored["ok"], true, "{restored}");
    assert_eq!(restored["stats"]["checked_functions"], 1);
    assert_eq!(restored["stats"]["reused_functions"], 1);
    let warm = client.result(json!({"id": 5, "op": "update"}));
    assert_eq!(warm["ok"], true, "{warm}");
    assert_eq!(warm["stats"]["reused_functions"], 2);
}

#[test]
fn tracks_extern_arrays_and_rejects_duplicate_paths_atomically() {
    let mut client = Client::start();
    let app = json!({"path": "app.clj", "source": CONSUMER});
    let store = json!({"path": "store.clj", "source": PROVIDER});
    let labels = json!({"path": "labels.edn", "source": EXTERNS});
    let counts = json!({"path": "counts.edn", "source": "{db/count [:-> number?]}"});
    let opened = client.result(json!({
        "id": 1, "op": "open", "files": [app, store], "externs": [labels, counts]
    }));
    assert_eq!(opened["ok"], true, "{opened}");
    assert_eq!(opened["files"].as_array().unwrap().len(), 4);
    let invalid = client.send(json!({
        "id": 2, "op": "update", "remove": ["store.clj"], "externs": [labels, labels]
    }));
    assert_eq!(invalid["error"]["code"], "invalid_request");
    let warm = client.result(json!({"id": 3, "op": "update"}));
    assert_eq!(warm["ok"], true, "{warm}");
    assert_eq!(warm["files"].as_array().unwrap().len(), 4);
    assert_eq!(warm["stats"]["parsed_files"], 0);

    let shortened = client.result(json!({"id": 4, "op": "update", "externs": [counts]}));
    assert_eq!(shortened["files"].as_array().unwrap().len(), 3);
    assert_eq!(shortened["stats"]["parsed_files"], 0);
    assert_eq!(shortened["files"][0]["diagnostics"][0]["code"], "PNX9003");
    let cleared = client.result(json!({"id": 5, "op": "update", "externs": []}));
    assert_eq!(cleared["files"].as_array().unwrap().len(), 2);

    let snapshot =
        client.result(json!({"id": 6, "files": [app, store], "externs": [labels, counts]}));
    assert_eq!(snapshot["ok"], true, "{snapshot}");
}

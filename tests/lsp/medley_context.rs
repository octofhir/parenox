use super::*;

const SOURCE: &str = include_str!("../fixtures/medley-context/requirements.clj");
const EXTERNS: &str = include_str!("../fixtures/medley-context/externs.edn");

#[test]
fn infers_medley_callback_parameters_from_dictionary_keys_and_values() {
    let fixture = Fixture::new("medley context");
    let path = fixture.0.join("src/requirements.clj");
    let externs = fixture.0.join("externs.edn");
    let document = uri(&path);
    fs::write(&path, SOURCE).unwrap();
    fs::write(&externs, EXTERNS).unwrap();
    fs::write(
        fixture.0.join("parenox.edn"),
        r#"{:externs ["externs.edn"]}"#,
    )
    .unwrap();
    let broken = SOURCE.replace(
        "(lib-requirements lib usages)",
        "(do (inc lib) (lib-requirements lib usages))",
    );
    let location = |text: &str, marker: &str, name: &str| {
        let (line, row) = text
            .lines()
            .enumerate()
            .find(|(_, row)| row.contains(marker))
            .unwrap();
        let character = row.find(marker).unwrap() + marker.find(name).unwrap();
        (line, character)
    };
    let hover = |id, marker, name| {
        let (line, character) = location(SOURCE, marker, name);
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/hover","params":{
            "textDocument":{"uri":document},"position":{"line":line,"character":character}
        }})
    };
    let change = |version, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let responses = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"languageId":"clojure","version":1,"text":SOURCE
        }}}),
        hover(2, " lib usages)", "lib"),
        hover(3, " lib usages)", "usages"),
        hover(4, "(subs %1", "%1"),
        hover(5, "(inc %2", "%2"),
        change(2, &broken),
        change(3, SOURCE),
        json!({"jsonrpc":"2.0","id":6,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    for (id, contents) in [
        (2, "lib\nSymbol"),
        (3, "usages\nVector<String>"),
        (4, "%1\nString"),
        (5, "%2\nNumber"),
    ] {
        let response = responses
            .iter()
            .find(|message| message["id"] == id)
            .unwrap();
        assert_eq!(
            response["result"]["contents"]["value"], contents,
            "{response}"
        );
    }
    let findings = |version| {
        let message = responses
            .iter()
            .find(|message| {
                message["method"] == "textDocument/publishDiagnostics"
                    && message["params"]["version"] == version
            })
            .unwrap();
        message["params"]["diagnostics"].as_array().unwrap()
    };
    assert!(findings(1).is_empty());
    assert!(findings(3).is_empty());
    let (line, character) = location(&broken, "(inc lib)", "lib");
    let mismatch = findings(2)
        .iter()
        .find(|diagnostic| diagnostic["code"] == "PNX2322")
        .unwrap();
    assert_eq!(
        mismatch["range"],
        json!({
            "start":{"line":line,"character":character},
            "end":{"line":line,"character":character+3}
        })
    );
    assert_eq!(
        mismatch["message"],
        "clojure.core/inc: expected Number, got Symbol"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), SOURCE);

    let check = || {
        Command::new(env!("CARGO_BIN_EXE_parenox"))
            .current_dir(&fixture.0)
            .args(["check", "--format", "json"])
            .output()
            .unwrap()
    };
    let good = check();
    assert!(
        good.status.success(),
        "{}",
        String::from_utf8_lossy(&good.stdout)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&good.stdout).unwrap()["complete"],
        true
    );
    fs::write(&path, &broken).unwrap();
    let bad = check();
    assert_eq!(bad.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&bad.stdout).unwrap();
    let file = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"].as_str().unwrap().ends_with("requirements.clj"))
        .unwrap();
    let bad = file["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["code"] == mismatch["code"])
        .unwrap();
    assert_eq!(bad["message"], mismatch["message"]);
    assert_eq!(bad["expected"], "Number");
    assert_eq!(bad["actual"], "Symbol");
    assert_eq!(bad["contract"]["name"], "clojure.core/inc");
    assert_eq!(bad["span"]["start"]["line"], line + 1);
    assert_eq!(bad["span"]["start"]["column"], character + 1);
    assert_eq!(fs::read_to_string(&externs).unwrap(), EXTERNS);
}

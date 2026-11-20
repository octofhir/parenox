use super::*;

const SOURCE: &str = include_str!("../fixtures/watch-diagnostics/watch_events.cljc");
const EXTERNS: &str = include_str!("../fixtures/watch-diagnostics/lsp4clj.edn");

#[test]
fn nested_watch_diagnostic_points_to_the_invalid_value_and_explains_the_path() {
    let fixture = Fixture::new("watch diagnostics");
    let path = fixture.0.join("src/watch_events.cljc");
    let document = uri(&path);
    fs::write(&path, SOURCE).unwrap();
    fs::write(fixture.0.join("lsp4clj.edn"), EXTERNS).unwrap();
    fs::write(
        fixture.0.join("parenox.edn"),
        r#"{:externs ["lsp4clj.edn"]}"#,
    )
    .unwrap();
    let wrong = SOURCE.replace("#_9 2", "#_9 4");
    let computed = wrong
        .replace("{:changes [", "(assoc {} :changes [")
        .replace("4}]}))", "4}])))");
    let selected = wrong.replace(
        r#"{:uri "file:///🦊.clj" :type #_9 4}"#,
        r#"#::coercer{:_/uri "file:///🦊.clj" :_/type #?(:clj 4 :cljs 2)}"#,
    );
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
        change(2, &wrong),
        change(3, &computed),
        change(4, &selected),
        change(5, SOURCE),
        json!({"jsonrpc":"2.0","id":2,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
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
    assert!(findings(5).is_empty());
    let leaf_message =
        "Argument 1 → field :changes → item 1 → field :type: expected 1 | 2 | 3, got 4";
    let origin = "Contract for watch-events/watched-files: :t/sig in ";
    for version in [2, 3, 4] {
        let [diagnostic] = findings(version).as_slice() else {
            panic!("unexpected diagnostics: {:?}", findings(version));
        };
        assert_eq!(diagnostic["code"], "PNX2322");
        let message = diagnostic["message"].as_str().unwrap();
        assert!(message.contains(leaf_message), "{message}");
        assert!(message.contains(origin), "{message}");
    }
    for (version, source, marker) in [(2, &wrong, "#_9 4"), (4, &selected, ":clj 4")] {
        let (line, row) = source
            .lines()
            .enumerate()
            .find(|(_, row)| row.contains(marker))
            .unwrap();
        let column = row[..row.find('4').unwrap()].encode_utf16().count();
        assert_eq!(
            findings(version)[0]["range"],
            json!({
                "start":{"line":line,"character":column},
                "end":{"line":line,"character":column+1}
            })
        );
    }
    assert_eq!(
        findings(3)[0]["range"]["start"],
        json!({"line":9,"character":4})
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), SOURCE);

    fs::write(&path, &wrong).unwrap();
    let checked = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .current_dir(&fixture.0)
        .args(["check", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(checked.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(report["complete"], true);
    let diag = &report["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| {
            file["path"]
                .as_str()
                .unwrap()
                .ends_with("watch_events.cljc")
        })
        .unwrap()["diagnostics"][0];
    let start = diag["span"]["start"]["offset"].as_u64().unwrap() as usize;
    let end = diag["span"]["end"]["offset"].as_u64().unwrap() as usize;
    assert_eq!(&wrong[start..end], "4");
    assert_eq!(diag["expected"], "1 | 2 | 3");
    assert_eq!(diag["actual"], "4");
    assert_eq!(diag["message"], leaf_message);
    assert_eq!(diag["contract"]["name"], "watch-events/watched-files");
    assert_eq!(
        diag["mismatch_path"],
        json!([
            {"kind":"argument","index":1},
            {"kind":"field","key":":changes"},
            {"kind":"item","index":1},
            {"kind":"field","key":":type"}
        ])
    );
}

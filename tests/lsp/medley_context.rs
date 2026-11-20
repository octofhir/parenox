use super::*;

const SOURCE: &str = include_str!("../fixtures/medley-context/requirements.clj");
const EXTERNS: &str = include_str!("../fixtures/medley-context/externs.edn");

fn project(name: &str, source: &str) -> (Fixture, PathBuf) {
    let fixture = Fixture::new(name);
    let path = fixture.0.join("src/requirements.clj");
    fs::write(&path, source).unwrap();
    fs::write(fixture.0.join("externs.edn"), EXTERNS).unwrap();
    fs::write(
        fixture.0.join("parenox.edn"),
        r#"{:externs ["externs.edn"]}"#,
    )
    .unwrap();
    (fixture, path)
}

fn location(text: &str, marker: &str, name: &str) -> (usize, usize) {
    let (line, row) = text
        .lines()
        .enumerate()
        .find(|(_, row)| row.contains(marker))
        .unwrap();
    let byte = row.find(marker).unwrap() + marker.find(name).unwrap();
    (line, row[..byte].encode_utf16().count())
}

#[test]
fn infers_medley_callback_parameters_from_dictionary_keys_and_values() {
    let (fixture, path) = project("medley context", SOURCE);
    let externs = fixture.0.join("externs.edn");
    let document = uri(&path);
    let broken = SOURCE.replace(
        "(lib-requirements lib usages)",
        "(do (inc lib) (lib-requirements lib usages))",
    );
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

#[test]
fn signature_help_shows_the_specialization_and_accounts_for_threaded_arguments() {
    let source = format!(
        "{SOURCE}
(defn repeated {{:t/sig '[(s/map-of symbol? number?) :-> number?]}} [initial]
  (loop [entries initial again true]
    (let [result (medley/map-kv-vals (fn [key n] (str key n)) entries)]
      (if again (recur {{\"x\" 1}} false) (count result)))))
(defn unknown-input {{:t/sig '[map? :-> t/any]}} [data]
  (medley/map-kv-vals (fn [k v] (str k v)) data))
(defn implicit-only {{:t/sig '[:-> t/any]}} []
  (-> label-entry (medley/map-kv-vals  )))
(defn choose
  {{:t/sig '[[string? :-> string?] [string? string? :-> string?]]}}
  ([first] first)
  ([first second] second))
(defn select-next {{:t/sig '[:-> string?]}} [] (choose \"v1\"  ))
"
    );
    let unfinished = format!("{source}(defn later [data] (medley/map-kv-vals ");
    let (fixture, path) = project("medley signature", &source);
    let document = uri(&path);
    let query = |id, (line, character)| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/signatureHelp","params":{
            "textDocument":{"uri":document},"position":{"line":line,"character":character}
        }})
    };
    let responses = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":document,"languageId":"clojure","version":1,"text":source
        }}}),
        query(2, location(&source, "(fn [lib usages]", "(fn")),
        query(
            3,
            location(&source, "#(str (subs %1 0) (inc %2)) labels", "labels"),
        ),
        query(
            4,
            location(&source, "(medley/map-kv-vals entries)", "entries"),
        ),
        query(5, location(&source, "(fn [key n]", "(fn")),
        query(6, location(&source, "(fn [k v]", "(fn")),
        query(7, location(&source, "(medley/map-kv-vals  )", " )")),
        query(8, location(&source, "(choose \"v1\"  )", " )")),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":document,"version":2},"contentChanges":[{"text":unfinished}]
        }}),
        query(
            9,
            (
                unfinished.lines().count() - 1,
                unfinished.lines().last().unwrap().len(),
            ),
        ),
        json!({"jsonrpc":"2.0","id":10,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let signature = |id| {
        &responses
            .iter()
            .find(|message| message["id"] == id)
            .unwrap()["result"]
    };
    for (id, active, callback, entries, result) in [
        (
            2,
            Some(0),
            "[Symbol, Vector<String> -> {:lib: String, :total: Number}]",
            "Map<Symbol, Vector<String>>",
            "Map<Symbol, {:lib: String, :total: Number}>",
        ),
        (
            3,
            Some(1),
            "[String, Number -> String]",
            "Map<String, Number>",
            "Map<String, String>",
        ),
        (
            4,
            Some(1),
            "[String, Number -> String]",
            "Map<String, Number>",
            "Map<String, String>",
        ),
        (5, Some(0), "[k, v -> r]", "Map<k, v>", "Map<k, r>"),
        (6, Some(0), "[k, v -> r]", "Map<k, v>", "Map<k, r>"),
        (7, None, "[k, v -> r]", "Map<k, v>", "Map<k, r>"),
        (9, None, "[k, v -> r]", "Map<k, v>", "Map<k, r>"),
    ] {
        let help = signature(id);
        let item = &help["signatures"][0];
        assert_eq!(help["activeParameter"], json!(active), "{help}");
        assert_eq!(
            item["parameters"],
            json!([{"label":callback},{"label":entries}])
        );
        assert_eq!(
            item["label"],
            format!("(medley.core/map-kv-vals [{callback} {entries}]) -> {result}")
        );
        assert!(
            item["documentation"]
                .as_str()
                .unwrap()
                .contains("extern declaration in ")
        );
        assert!(
            item["documentation"]
                .as_str()
                .unwrap()
                .ends_with("externs.edn")
        );
    }
    let overload = signature(8);
    assert_eq!(overload["activeSignature"], 1, "{overload}");
    assert_eq!(overload["activeParameter"], 1, "{overload}");
    assert_eq!(
        overload["signatures"][1]["label"],
        "(choose [first second])"
    );
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

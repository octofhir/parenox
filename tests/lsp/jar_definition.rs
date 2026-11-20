use super::*;
use parenox_lsp::text::{Lines, Range};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

const ARCHIVE_SOURCE: &str = concat!(
    "(ns lab.external)\n",
    "(defn public [value] value)\n",
    "(defn- hidden [value] value)\n",
    "(def SomeValue :constant)\n",
    "(do \"🧪\" (defn after-emoji [] :emoji))\n",
);
const SOURCE: &str = concat!(
    "(ns demo\n",
    "  (:require [lab.external :as e :refer [public]]\n",
    "            [clojure.string :as str :refer [join]]))\n",
    "(defn use-jar [xs]\n",
    "  [\"🧪\" (e/public :x) (public :x) (lab.external/public :x)\n",
    "   (e/after-emoji) (e/hidden :x) (e/not-present)\n",
    "   (str/join \",\" xs) (join \",\" xs) (map inc xs) (clojure.core/map inc xs)])\n",
    "(defn shadow [public x] (public x))\n",
);

fn archive_project(root: &Path) -> PathBuf {
    let jar = root.join("lib/deps % #.jar");
    fs::create_dir(jar.parent().unwrap()).unwrap();
    let mut archive = ZipWriter::new(fs::File::create(&jar).unwrap());
    archive
        .start_file(
            "lab/external.clj",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .unwrap();
    archive.write_all(ARCHIVE_SOURCE.as_bytes()).unwrap();
    archive.finish().unwrap();
    let deps = include_str!("../../deps.edn").replacen(
        ":deps {",
        ":paths [\"src\"] :deps {lab/external {:local/root \"lib/deps % #.jar\"} ",
        1,
    );
    fs::write(root.join("deps.edn"), deps).unwrap();
    fs::write(root.join("src/demo.clj"), SOURCE).unwrap();
    jar
}

fn expected_jar_uri(jar: &Path) -> String {
    let archive = uri(jar).replace("deps%20%%20#.jar", "deps%20%25%20%23.jar");
    format!("jar:{archive}!/lab/external.clj")
}

fn candidate(root: &Path, classpath: &[u8]) -> Child {
    let path = root.join("classpath.txt");
    fs::write(&path, classpath).unwrap();
    let binary = std::env::var_os("PARENOX_LSP_CANDIDATE")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_parenox").into());
    lsp_server_args(
        Path::new(&binary),
        ["lsp", "--classpath", path.to_str().unwrap()],
    )
}

fn cursor(source: &str, line: usize, needle: &str, offset: usize) -> Value {
    let row = source.lines().nth(line).unwrap();
    let start = row.find(needle).unwrap();
    let character = row[..start].encode_utf16().count() + offset;
    json!({"line":line,"character":character})
}

fn location(file: &str, line: usize, start: usize, end: usize) -> Value {
    json!({"uri":file,"range":{
        "start":{"line":line,"character":start},"end":{"line":line,"character":end}
    }})
}

#[test]
fn jar_definition_resolves_public_symbols_and_round_trips_escaped_uri() {
    let fixture = Fixture::new("jar definition");
    let root = fixture.0.canonicalize().unwrap();
    let jar = archive_project(&root);
    let target = expected_jar_uri(&jar);
    let classpath = std::env::join_paths([Path::new("src"), &jar]).unwrap();
    let result = probe(
        candidate(&root, classpath.as_encoded_bytes()),
        &root,
        &target,
        false,
    );
    let namespace = location(&target, 0, 4, 16);
    let public = location(&target, 1, 6, 12);
    let after_emoji = location(&target, 4, 15, 26);
    for label in ["namespace", "alias declaration"] {
        assert_eq!(result[label], namespace, "{label}");
    }
    for label in [
        "refer declaration",
        "alias part",
        "member",
        "referred",
        "qualified",
    ] {
        assert_eq!(result[label], public, "{label}");
    }
    assert_eq!(result["after emoji"], after_emoji);
    assert_eq!(result["custom overlay"], after_emoji);
    let source_file = uri(&root.join("src/demo.clj"));
    assert_eq!(result["local shadow"], location(&source_file, 7, 14, 20));
    assert_eq!(result["declaration"], location(&source_file, 1, 30, 31));
    assert!(result["private"].is_null());
    assert!(result["missing"].is_null());
    assert_eq!(result["archive contents"], ARCHIVE_SOURCE);
    assert_eq!(
        fs::read_to_string(root.join("src/demo.clj")).unwrap(),
        SOURCE
    );
}

#[test]
#[ignore = "requires Clojure CLI and a Clojure LSP executable"]
fn jar_definition_matches_pinned_clojure_lsp() {
    let fixture = Fixture::new("jar definition parity");
    let root = fixture.0.canonicalize().unwrap();
    let jar = archive_project(&root);
    let target = expected_jar_uri(&jar);
    let classpath = Command::new("clojure")
        .arg("-Spath")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        classpath.status.success(),
        "{}",
        String::from_utf8_lossy(&classpath.stderr)
    );
    let classpath = String::from_utf8(classpath.stdout).unwrap();
    let reference =
        std::env::var_os("PARENOX_REFERENCE_LSP").unwrap_or_else(|| "clojure-lsp".into());
    let reference = Command::new(reference)
        .arg("listen")
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let reference = probe(reference, &root, &target, true);
    let candidate = probe(
        candidate(&root, classpath.trim().as_bytes()),
        &root,
        &target,
        true,
    );
    for label in [
        "namespace",
        "alias declaration",
        "refer declaration",
        "alias part",
        "member",
        "referred",
        "qualified",
        "after emoji",
        "local shadow",
        "declaration",
        "string join",
        "string referred",
        "core map",
        "core qualified",
        "core contents",
        "string contents",
        "string overlay",
        "custom overlay",
    ] {
        assert_eq!(candidate[label], reference[label], "{label}");
        assert!(!candidate[label].is_null(), "{label}");
    }
    assert_eq!(candidate["archive contents"], ARCHIVE_SOURCE);
    assert!(candidate["private"].is_null());
    assert!(candidate["missing"].is_null());
    for (label, contents, name) in [
        ("core map", "core contents", "map"),
        ("string join", "string contents", "join"),
        ("string overlay", "string contents", "split"),
    ] {
        let source = candidate[contents].as_str().unwrap();
        let range: Range = serde_json::from_value(candidate[label]["range"].clone()).unwrap();
        let span = Lines::new(source).byte_range(range).unwrap();
        assert_eq!(&source[span], name);
    }
}

fn probe(
    mut child: Child,
    root: &Path,
    target: &str,
    include_jvm: bool,
) -> std::collections::BTreeMap<&'static str, Value> {
    let file = uri(&root.join("src/demo.clj"));
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut next_id = 0;
    let mut exchange = |method: &str, params: Value, request: bool| {
        let mut message = json!({"jsonrpc":"2.0","method":method,"params":params});
        if request {
            next_id += 1;
            message["id"] = json!(next_id);
        }
        send_message(&mut input, &message);
        if !request {
            return Value::Null;
        }
        loop {
            let reply = read_message(&mut output).expect("LSP message");
            if reply["id"] == next_id && reply["method"].is_null() {
                return reply;
            }
            if !reply["id"].is_null() && !reply["method"].is_null() {
                send_message(
                    &mut input,
                    &json!({"jsonrpc":"2.0","id":reply["id"],"result":null}),
                );
            }
        }
    };
    let initialize = exchange(
        "initialize",
        json!({
            "rootUri":uri(root),"initializationOptions":{"dependencyScheme":"jar"},
            "capabilities":{"textDocument":{"definition":{"linkSupport":true}}}
        }),
        true,
    );
    assert_eq!(
        initialize["result"]["capabilities"]["definitionProvider"],
        true
    );
    exchange("initialized", json!({}), false);
    exchange(
        "textDocument/didOpen",
        json!({"textDocument":{
            "uri":file,"languageId":"clojure","version":1,"text":SOURCE
        }}),
        false,
    );

    let mut result = std::collections::BTreeMap::new();
    for (label, line, needle, offset) in [
        ("namespace", 1, "lab.external", 3),
        ("alias declaration", 1, "as e", 3),
        ("refer declaration", 1, "public", 2),
        ("alias part", 4, "e/public", 0),
        ("member", 4, "e/public", 4),
        ("referred", 4, "(public", 3),
        ("qualified", 4, "lab.external/public", 15),
        ("after emoji", 5, "e/after-emoji", 6),
        ("private", 5, "e/hidden", 4),
        ("missing", 5, "e/not-present", 4),
        ("local shadow", 7, "(public", 3),
    ] {
        let response = exchange(
            "textDocument/definition",
            json!({
                "textDocument":{"uri":file},"position":cursor(SOURCE,line,needle,offset)
            }),
            true,
        );
        result.insert(label, single_location(&response));
    }
    let declaration = exchange(
        "textDocument/declaration",
        json!({
            "textDocument":{"uri":file},"position":cursor(SOURCE,4,"e/public",4)
        }),
        true,
    );
    result.insert("declaration", single_location(&declaration));
    let contents = exchange("clojure/dependencyContents", json!({"uri":target}), true);
    result.insert("archive contents", contents["result"].clone());

    if include_jvm {
        for (label, needle, offset) in [
            ("string join", "str/join", 5),
            ("string referred", "(join", 2),
            ("core map", "(map", 2),
            ("core qualified", "clojure.core/map", 14),
        ] {
            let response = exchange(
                "textDocument/definition",
                json!({
                    "textDocument":{"uri":file},"position":cursor(SOURCE,6,needle,offset)
                }),
                true,
            );
            result.insert(label, single_location(&response));
        }
        for (label, definition) in [
            ("core contents", "core map"),
            ("string contents", "string join"),
        ] {
            let uri = result[definition]["uri"].as_str().unwrap_or_else(|| {
                panic!("missing location for {definition}: {}", result[definition])
            });
            let response = exchange("clojure/dependencyContents", json!({"uri":uri}), true);
            assert!(response["error"].is_null(), "{response}");
            result.insert(label, response["result"].clone());
        }
    }

    let overlay = SOURCE
        .replace("e/public :x", "e/after-emoji")
        .replace("str/join", "str/split");
    exchange(
        "textDocument/didChange",
        json!({
            "textDocument":{"uri":file,"version":7},"contentChanges":[{"text":overlay}]
        }),
        false,
    );
    for (label, previous, line, reference, offset) in [
        ("custom overlay", "member", 4, "e/after-emoji", 6),
        ("string overlay", "string join", 6, "str/split", 5),
    ]
    .into_iter()
    .take(if include_jvm { 2 } else { 1 })
    {
        let position = cursor(&overlay, line, reference, offset);
        let mut definition = Value::Null;
        // The reference analyzes document changes asynchronously.
        for _ in 0..20 {
            let response = exchange(
                "textDocument/definition",
                json!({
                    "textDocument":{"uri":file},"position":position
                }),
                true,
            );
            definition = single_location(&response);
            if !definition.is_null() && definition != result[previous] {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        result.insert(label, definition);
    }
    exchange("shutdown", Value::Null, true);
    exchange("exit", Value::Null, false);
    drop(input);
    assert!(child.wait().unwrap().success());
    result
}

fn single_location(response: &Value) -> Value {
    assert!(response["error"].is_null(), "{response}");
    let location = &response["result"];
    if let Some(items) = location.as_array() {
        match items.as_slice() {
            [] => Value::Null,
            [item] => item.clone(),
            _ => panic!("expected a single location: {response}"),
        }
    } else {
        location.clone()
    }
}

use super::java_definition::{
    class_file, definition_location, java_definition_request, java_lsp, read_contents,
    write_archive,
};
use super::*;

const SOURCE: &str = concat!(
    "(ns demo.hints\n",
    "  (:import java.util.UUID))\n",
    "(defn parameters [^UUID x ^java.util.UUID y] [x y])\n",
    "(defn returning ^UUID [x] x)\n",
    "(defn tagged [^{:tag java.util.UUID :custom UUID} x ^String y] [x y])\n",
    "(defn expression [x] [\"🧪\" ^UUID x ^{:tag java.util.UUID} (identity x)])\n",
    "(defn shadow [UUID] (let [^UUID x nil] [x UUID]))\n",
    "(defn arbitrary [^{:nested {:tag UUID}} x ^long n] [x n])\n",
    "(defn quoted [] ['^UUID x '(let [^UUID y nil] y) #_(let [^UUID z nil] z) nil])\n",
    "(defmacro as-data [arg] (list 'quote arg))\n",
    "(defn opaque [x] (as-data ^UUID x))\n",
    "(defn ^UUID function-hint [x] x)\n",
    "(defn multi (^UUID [x] x) (^java.util.UUID [x y] [x y]))\n",
    "(defn dot [x] [(.getLeastSignificantBits ^UUID x) (. ^UUID x getLeastSignificantBits)])\n",
    "(defn discard-tag [^{:tag #_java.lang.String UUID :ignore UUID} x] x)\n",
    "(defn stacked [x] [^UUID ^java.util.ArrayList x ^:private ^UUID x])\n",
    "(defn ^UUID overridden {:tag java.util.ArrayList} [x] x)\n",
);
const SOURCES: [(&str, &str); 3] = [
    (
        "java/util/UUID",
        "package java.util;\npublic final class UUID {}\n",
    ),
    (
        "java/util/ArrayList",
        "package java.util;\n// 🦀\npublic class ArrayList {}\n",
    ),
    (
        "java/lang/String",
        "package java.lang;\npublic final class String {}\n",
    ),
];

#[test]
fn java_type_hints_follow_metadata_roles_and_unsaved_imports() {
    let fixture = Fixture::new("java type hints");
    java_type_hints(&fixture, false);
}

#[test]
#[ignore = "requires Clojure CLI and a Clojure LSP executable"]
fn java_type_hints_match_pinned_reference_for_supported_positions() {
    let fixture = Fixture::new("java type hints parity");
    java_type_hints(&fixture, true);
    java_type_hints(&fixture, false);
}

fn java_type_hints(fixture: &Fixture, reference: bool) {
    let root = fixture.0.canonicalize().unwrap();
    let path = root.join("src/demo/hints.clj");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, SOURCE).unwrap();
    let file = uri(&path);
    let overlay = SOURCE
        .replace("(:import java.util.UUID)", "(:import java.util.ArrayList)")
        .replace("\"🧪\" ^UUID", "\"🧪🦀\" ^ArrayList");
    let home = root.join("Synthetic JDK");
    fs::create_dir_all(home.join("jmods")).unwrap();
    fs::create_dir_all(home.join("lib")).unwrap();
    write_archive(
        &home.join("jmods/java.base.jmod"),
        SOURCES.map(|(name, _)| (format!("classes/{name}.class"), class_file(name, true))),
    );
    let archive = home.join("lib/src.zip");
    write_archive(
        &archive,
        SOURCES.map(|(name, source)| (format!("java.base/{name}.java"), source)),
    );
    let cache = root.join("reference-cache");
    let mut child = if reference {
        let config = root.join(".lsp");
        fs::create_dir_all(&config).unwrap();
        fs::write(config.join("config.edn"), format!(
            "{{:hover {{:clojuredocs false}} :log-path {:?} :java {{:jdk-source-uri {:?} :download-jdk-source? false}}}}",
            root.join("reference.log"), archive,
        )).unwrap();
        Command::new(
            std::env::var_os("PARENOX_REFERENCE_LSP").unwrap_or_else(|| "clojure-lsp".into()),
        )
        .arg("listen")
        .current_dir(&root)
        .env("XDG_CACHE_HOME", &cache)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap()
    } else {
        java_lsp(&root, &home, None)
    };
    let source_uris = SOURCES.map(|(name, _)| {
        let resource = format!("java.base/{name}.java");
        if reference {
            uri(&cache.join("clojure-lsp/jdk").join(resource))
        } else {
            format!("jar:{}!/{resource}", uri(&archive))
        }
    });
    let [uuid, list, string] = source_uris
        .each_ref()
        .map(|target| definition_location(target, 0, 0, 0));
    let local = definition_location(&file, 6, 14, 18);
    let null = Value::Null;
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut id = 0;
    let mut exchange = |mut message: Value| {
        if message.get("id").is_none() {
            send_message(&mut input, &message);
            return Value::Null;
        }
        id += 1;
        message["id"] = json!(id);
        send_message(&mut input, &message);
        loop {
            let reply = read_message(&mut output).expect("LSP response");
            if reply["id"] == id && reply["method"].is_null() {
                assert!(reply["error"].is_null(), "{message}: {reply}");
                return reply["result"].clone();
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
        json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
            "rootUri":uri(&root),"initializationOptions":{"dependencyScheme":"jar"},
            "capabilities":{"textDocument":{"definition":{"linkSupport":true}}}
        }}),
    );
    assert_eq!(initialize["capabilities"]["definitionProvider"], true);
    exchange(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
    exchange(
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":file,"version":1,"languageId":"clojure","text":SOURCE
        }}}),
    );
    let verify = |exchange: &mut dyn FnMut(Value) -> Value,
                  source: &str,
                  query,
                  expected: &Value,
                  wait: bool| {
        let request = java_definition_request(0, &file, source, query);
        let mut actual = exchange(request.clone());
        if reference && wait {
            // Source extraction and didChange indexing can finish after the response.
            for _ in 0..60 {
                if actual == *expected {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
                actual = exchange(request.clone());
            }
        }
        assert_eq!(&actual, expected, "{query:?}");
    };
    for (line, needle, offset, expected) in [
        (1, "java.util.UUID", 12, &uuid),
        (2, "^UUID", 0, &null),
        (2, "^UUID", 2, &uuid),
        (2, "^java.util.UUID", 3, &uuid),
        (3, "^UUID", 3, &uuid),
        (4, "tag java.util.UUID", 16, &uuid),
        (4, "custom UUID", 8, if reference { &uuid } else { &null }),
        (4, "^String", 2, &string),
        (5, "^UUID", 2, &uuid),
        (5, "tag java.util.UUID", 16, &uuid),
        (6, "^UUID", 2, if reference { &local } else { &uuid }),
        (6, "[x UUID]", 4, &local),
        (7, "^long", 2, &null),
        (8, "'^UUID", 3, &null),
        (8, "'(let [^UUID", 9, &null),
        (8, "#_(let [^UUID", 10, &null),
        (10, "^UUID", 2, if reference { &uuid } else { &null }),
        (11, "^UUID", 2, &uuid),
        (12, "^UUID", 2, &uuid),
        (12, "^java.util.UUID", 13, &uuid),
        (13, "Bits ^UUID", 7, &uuid),
        (13, "(. ^UUID", 5, &uuid),
        (14, "java.lang.String", 12, &null),
        (14, "String UUID", 8, &uuid),
        (14, "ignore UUID", 8, if reference { &uuid } else { &null }),
        (15, "^UUID", 2, &uuid),
        (
            15,
            "^java.util.ArrayList",
            13,
            if reference { &list } else { &null },
        ),
        (15, "^:private ^UUID", 12, &uuid),
    ] {
        verify(
            &mut exchange,
            SOURCE,
            (line, needle, offset),
            expected,
            line == 1,
        );
    }
    if !reference {
        verify(&mut exchange, SOURCE, (7, "tag UUID", 5), &null, false);
        verify(&mut exchange, SOURCE, (16, "^UUID", 2), &null, false);
    }
    exchange(
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":7},"contentChanges":[{"text":overlay}]
        }}),
    );
    for (line, needle, offset, expected) in [
        (1, "ArrayList", 3, &list),
        (5, "^ArrayList", 3, &list),
        (5, "tag java.util.UUID", 16, &uuid),
    ] {
        verify(
            &mut exchange,
            &overlay,
            (line, needle, offset),
            expected,
            line == 1,
        );
    }
    if !reference {
        verify(&mut exchange, &overlay, (3, "^UUID", 3), &null, false);
    }
    for (target, (_, source)) in source_uris.iter().zip(SOURCES) {
        assert_eq!(exchange(read_contents(0, target)), source);
    }
    assert_eq!(fs::read_to_string(path).unwrap(), SOURCE);
    exchange(json!({"jsonrpc":"2.0","id":0,"method":"shutdown"}));
    exchange(json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
}

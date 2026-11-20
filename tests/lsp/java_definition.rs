use super::*;
use zip::{ZipWriter, write::SimpleFileOptions};

const UUID: &str = "package java.util;\n\n// UUID source 🧪\npublic final class UUID {}\n";
const ARRAY_LIST: &str = "package java.util;\npublic class ArrayList {}\n";
const STRING: &str = "package java.lang;\npublic final class String {}\n";

pub(super) fn java_definition_request(
    id: usize,
    file: &str,
    source: &str,
    query: (usize, &str, usize),
) -> Value {
    let (line, needle, delta) = query;
    let row = source.lines().nth(line).unwrap();
    let character = row[..row.find(needle).unwrap()].encode_utf16().count() + delta;
    json!({"jsonrpc":"2.0","id":id,"method":"textDocument/definition","params":{
        "textDocument":{"uri":file},"position":{"line":line,"character":character}
    }})
}

pub(super) fn definition_location(uri: &str, line: usize, start: usize, end: usize) -> Value {
    json!({"uri":uri,"range":{
        "start":{"line":line,"character":start},"end":{"line":line,"character":end}
    }})
}

pub(super) fn read_contents(id: usize, uri: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"clojure/dependencyContents","params":{"uri":uri}})
}

pub(super) fn write_archive<Name: AsRef<str>, Bytes: AsRef<[u8]>>(
    path: &Path,
    files: impl IntoIterator<Item = (Name, Bytes)>,
) {
    let mut archive = ZipWriter::new(fs::File::create(path).unwrap());
    for (name, text) in files {
        archive
            .start_file(name.as_ref(), SimpleFileOptions::default())
            .unwrap();
        archive.write_all(text.as_ref()).unwrap();
    }
    archive.finish().unwrap();
}

pub(super) fn java_lsp(root: &Path, home: &Path, classpath: Option<&Path>) -> Child {
    let mut child = Command::new(env!("CARGO_BIN_EXE_parenox"));
    child.args(["lsp", "--clojuredocs-offline"]);
    if let Some(classpath) = classpath {
        child.arg("--classpath").arg(classpath);
    }
    child
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("JAVA_HOME", home)
        .env("PATH", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

#[test]
fn java_definition_resolves_only_class_positions_in_the_selected_jdk_sources() {
    let fixture = Fixture::new("jdk class definition");
    let root = fixture.0.canonicalize().unwrap();
    let home = root.join("Synthetic JDK");
    fs::create_dir_all(home.join("jmods")).unwrap();
    fs::create_dir_all(home.join("lib")).unwrap();
    let jmod = home.join("jmods/java.base.jmod");
    write_archive(
        &jmod,
        [
            ("java/util/UUID", true),
            ("java/util/ArrayList", true),
            ("java/lang/String", true),
            ("java/util/Hidden", false),
            ("java/util/NoSource", true),
        ]
        .map(|(class, public)| (format!("classes/{class}.class"), class_file(class, public))),
    );
    let src_zip = home.join("lib/src.zip");
    write_archive(
        &src_zip,
        [
            ("java.base/java/util/UUID.java", UUID),
            ("java.base/java/util/ArrayList.java", ARRAY_LIST),
            ("java.base/java/lang/String.java", STRING),
            (
                "java.base/java/util/Hidden.java",
                "package java.util; class Hidden {}",
            ),
            (
                "java.base/java/util/Orphan.java",
                "package java.util; public class Orphan {}",
            ),
            ("java.base/README", "not a class source"),
        ],
    );

    let plain = concat!(
        "(ns demo.plain\n",
        "  (:import java.util.UUID))\n",
        "(defn uuid [value] [\"🧪\" (UUID/randomUUID) (java.util.UUID/randomUUID) (UUID/.toString value) (java.util.UUID/.toString value)])\n",
        "(defn implicit [value] (String/.length value))\n",
        "(defn shadow [UUID value] [UUID (UUID/.toString value)])\n",
        "(defn quoted [] '(UUID/randomUUID))\n",
        "(defn discarded [] #_(UUID/randomUUID) nil)\n",
        "[(java.util.Hidden/value) (java.util.Orphan/value) (java.util.NoSource/value)]\n",
        "(def id (UUID/randomUUID))\n",
        "(let [x 1] (UUID/randomUUID))\n",
    );
    let live = plain
        .replacen(
            "(:import java.util.UUID)",
            "(:import java.util.ArrayList)",
            1,
        )
        .replacen("(UUID/randomUUID)", "(ArrayList/.size value)", 1);
    let grouped_vector = "(ns demo.grouped-vector\n  (:import [java.util UUID ArrayList]))\n";
    let grouped_list = "(ns demo.grouped-list\n  (:import (java.util UUID)))\n";
    let alias = concat!(
        "(ns demo.alias (:require [demo.fake :as UUID]) (:import java.util.UUID))\n",
        "(UUID/randomUUID)\n",
        "(UUID/missing)\n",
    );
    let project_source = root.join("src/demo");
    fs::create_dir_all(&project_source).unwrap();
    let path = |name| project_source.join(format!("{name}.clj"));
    for (name, text) in [
        ("plain", plain),
        ("grouped_vector", grouped_vector),
        ("grouped_list", grouped_list),
        ("alias", alias),
        ("fake", "(ns demo.fake)\n(defn randomUUID [] :clojure)\n"),
        ("unrelated", "(ns demo.unrelated)\n"),
    ] {
        fs::write(path(name), text).unwrap();
    }
    let file = uri(&path("plain"));
    let vector_file = uri(&path("grouped_vector"));
    let list_file = uri(&path("grouped_list"));
    let alias_file = uri(&path("alias"));
    let archive_uri = format!("jar:{}!/java.base", uri(&src_zip));
    let uuid_uri = format!("{archive_uri}/java/util/UUID.java");
    let list_uri = format!("{archive_uri}/java/util/ArrayList.java");
    let string_uri = format!("{archive_uri}/java/lang/String.java");

    let mut requests = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&root)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":file,"version":1,"languageId":"clojure","text":plain
        }}}),
    ];
    let cases = [
        (&file, plain, (1, "java.util.UUID", 2)),
        (&file, plain, (1, "java.util.UUID", 12)),
        (&file, plain, (2, "UUID/randomUUID", 4)),
        (&file, plain, (2, "UUID/randomUUID", 8)),
        (&file, plain, (2, "java.util.UUID/randomUUID", 12)),
        (&file, plain, (2, "java.util.UUID/randomUUID", 18)),
        (&file, plain, (2, "UUID/.toString", 2)),
        (&file, plain, (2, "UUID/.toString", 9)),
        (&file, plain, (2, "java.util.UUID/.toString", 12)),
        (&file, plain, (2, "java.util.UUID/.toString", 18)),
        (&file, plain, (3, "String/.length", 2)),
        (&file, plain, (3, "String/.length", 10)),
        (&file, plain, (4, "[UUID (", 2)),
        (&file, plain, (4, "UUID/.toString", 1)),
        (&file, plain, (5, "UUID/randomUUID", 2)),
        (&file, plain, (6, "UUID/randomUUID", 2)),
        (&file, plain, (7, "java.util.Hidden", 12)),
        (&file, plain, (7, "java.util.Orphan", 12)),
        (&file, plain, (7, "java.util.NoSource", 12)),
        (&vector_file, grouped_vector, (1, "java.util", 3)),
        (&vector_file, grouped_vector, (1, "UUID", 2)),
        (&vector_file, grouped_vector, (1, "ArrayList", 2)),
        (&list_file, grouped_list, (1, "java.util", 3)),
        (&list_file, grouped_list, (1, "UUID", 2)),
        (&alias_file, alias, (1, "UUID/randomUUID", 2)),
        (&alias_file, alias, (2, "UUID/missing", 2)),
        (&file, plain, (8, "UUID/randomUUID", 2)),
        (&file, plain, (9, "UUID/randomUUID", 2)),
    ];
    requests.extend(
        cases
            .into_iter()
            .enumerate()
            .map(|(index, (file, source, query))| {
                java_definition_request(index + 2, file, source, query)
            }),
    );
    requests.extend([
        read_contents(30, &uuid_uri),
        read_contents(31, &list_uri),
        read_contents(32, &string_uri),
        read_contents(33, &format!("{archive_uri}/java/util/Orphan.java")),
        read_contents(34, &format!("{archive_uri}/java/util/Hidden.java")),
        read_contents(35, &format!("{archive_uri}/README")),
        read_contents(36, &format!("{archive_uri}/java%2Futil%2FUUID.java")),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":7},"contentChanges":[{"text":live}]
        }}),
        java_definition_request(37, &file, &live, (1, "java.util.ArrayList", 12)),
        java_definition_request(38, &file, &live, (2, "ArrayList/.size", 2)),
        java_definition_request(39, &file, &live, (2, "UUID/.toString", 2)),
        java_definition_request(40, &file, &live, (2, "java.util.UUID/.toString", 12)),
        json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles","params":{
            "changes":[{"uri":uri(&path("unrelated")),"type":2}]
        }}),
        read_contents(41, &uuid_uri),
        json!({"jsonrpc":"2.0","id":42,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let replies = transcript_with(java_lsp(&root, &home, None), &requests);
    let result = |id| {
        let reply = replies.iter().find(|message| message["id"] == id).unwrap();
        assert!(reply["error"].is_null(), "{reply}");
        &reply["result"]
    };
    for (ids, target) in [
        (&[2, 3, 4, 6, 8, 10, 15, 22, 25, 28, 29, 40][..], &uuid_uri),
        (&[23, 37, 38], &list_uri),
        (&[12], &string_uri),
    ] {
        for id in ids {
            assert_eq!(
                result(*id),
                &definition_location(target, 0, 0, 0),
                "request {id}"
            );
        }
    }
    assert_eq!(result(14), &definition_location(&file, 4, 14, 18));
    assert_eq!(
        result(26),
        &definition_location(&uri(&path("fake")), 1, 6, 16)
    );
    for id in [
        5, 7, 9, 11, 13, 16, 17, 18, 19, 20, 21, 24, 27, 33, 34, 35, 36, 39,
    ] {
        assert!(result(id).is_null(), "request {id}: {}", result(id));
    }
    for (id, text) in [(30, UUID), (31, ARRAY_LIST), (32, STRING), (41, UUID)] {
        assert_eq!(result(id), text, "request {id}");
    }
    assert_eq!(fs::read_to_string(path("plain")).unwrap(), plain);
}

#[test]
fn java_definition_requires_sources_for_the_winning_jdk_class() {
    let fixture = Fixture::new("jdk class provenance");
    let root = fixture.0.canonicalize().unwrap();
    let home = root.join("Synthetic JDK");
    fs::create_dir_all(home.join("jmods")).unwrap();
    fs::create_dir_all(home.join("lib")).unwrap();
    let jmod = home.join("jmods/java.base.jmod");
    write_archive(
        &jmod,
        [
            (
                "classes/java/util/UUID.class",
                class_file("java/util/UUID", true),
            ),
            (
                "classes/javax/example/Choice.class",
                class_file("javax/example/Choice", true),
            ),
            (
                "classes/java/util/Unreadable.class",
                class_file("java/util/Unreadable", true),
            ),
        ],
    );
    let sources = home.join("lib/src.zip");
    write_archive(
        &sources,
        [
            ("java.base/java/util/UUID.java", UUID.as_bytes()),
            (
                "java.base/javax/example/Choice.java",
                b"package javax.example; public class Choice {}",
            ),
            ("java.base/java/util/Unreadable.java", b"\xff\n"),
        ],
    );
    let dependency = root.join("classes.jar");
    write_archive(
        &dependency,
        [
            ("java/util/UUID.class", class_file("java/util/UUID", false)),
            (
                "javax/example/Choice.class",
                class_file("javax/example/Choice", true),
            ),
        ],
    );
    let classpath = root.join("classpath.txt");
    fs::write(
        &classpath,
        std::env::join_paths([Path::new("src"), &dependency])
            .unwrap()
            .as_encoded_bytes(),
    )
    .unwrap();
    let path = root.join("src/demo.clj");
    let source = "(ns demo (:import [javax.example Choice] [java.util UUID Unreadable]))\n[(Choice/probe) (UUID/randomUUID) (Unreadable/probe)]\n";
    fs::write(&path, source).unwrap();
    let file = uri(&path);
    let uuid_uri = format!("jar:{}!/java.base/java/util/UUID.java", uri(&sources));
    let choice_uri = format!("jar:{}!/java.base/javax/example/Choice.java", uri(&sources));
    let unreadable_uri = format!("jar:{}!/java.base/java/util/Unreadable.java", uri(&sources));
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&root)}}),
        read_contents(2, &uuid_uri),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":file,"version":1,"languageId":"clojure","text":source
        }}}),
        java_definition_request(3, &file, source, (1, "Choice/probe", 2)),
        java_definition_request(4, &file, source, (1, "UUID/randomUUID", 2)),
        read_contents(5, &choice_uri),
        java_definition_request(6, &file, source, (1, "Unreadable/probe", 2)),
        read_contents(7, &unreadable_uri),
        json!({"jsonrpc":"2.0","id":8,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ];
    let query = || {
        let replies = transcript_with(java_lsp(&root, &home, Some(&classpath)), &requests);
        [2, 3, 4, 5, 6, 7].map(|id| {
            let reply = replies.iter().find(|message| message["id"] == id).unwrap();
            assert!(reply["error"].is_null(), "{reply}");
            reply["result"].clone()
        })
    };
    assert_eq!(
        query(),
        [
            json!(UUID),
            Value::Null,
            definition_location(&uuid_uri, 0, 0, 0),
            Value::Null,
            Value::Null,
            Value::Null,
        ]
    );
    fs::rename(&sources, home.join("lib/sources-for-first-run.zip")).unwrap();
    assert_eq!(query(), [const { Value::Null }; 6]);
}

pub(super) fn class_file(name: &str, public: bool) -> Vec<u8> {
    // Minimal Java 21 class header and constant pool. Definition needs identity and visibility.
    let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 65, 0, 5];
    for (index, text) in [(1u16, name), (3, "java/lang/Object")] {
        bytes.push(1); // CONSTANT_Utf8
        bytes.extend(u16::try_from(text.len()).unwrap().to_be_bytes());
        bytes.extend(text.as_bytes());
        bytes.push(7); // CONSTANT_Class
        bytes.extend(index.to_be_bytes());
    }
    let flags = if public { 0x0021u16 } else { 0x0020 };
    let header = [flags, 2, 4, 0, 0, 0, 0];
    bytes.extend(header.into_iter().flat_map(u16::to_be_bytes));
    bytes
}

#[test]
fn java_definition_resolves_evaluated_classes_and_constructor_operands() {
    let fixture = Fixture::new("java class positions");
    java_class_positions(&fixture, false);
}

#[test]
#[ignore = "requires Clojure CLI and a Clojure LSP executable"]
fn java_definition_class_positions_match_pinned_reference() {
    let fixture = Fixture::new("java class positions parity");
    java_class_positions(&fixture, true);
    java_class_positions(&fixture, false);
}

fn java_class_positions(fixture: &Fixture, reference: bool) {
    const SOURCE: &str = concat!(
        "(ns demo.classes\n",
        "  (:import [java.util UUID]))\n",
        "(defn bare [x] [\"🧪\" UUID java.util.UUID (instance? UUID x) (instance? java.util.UUID x) String])\n",
        "(defn ctor [] [\"🧪\" (UUID. 0 0) (java.util.UUID. 0 0) (new UUID 0 0) (new java.util.UUID 0 0)])\n",
        "(defn shadow [UUID x] [UUID (UUID. 0 0) (new UUID 0 0) (instance? UUID x)])\n",
        "(defn quoted [] ['UUID 'java.util.UUID '(UUID. 0 0) (quote UUID) #_(new UUID 0 0) nil])\n",
        "(defmacro as-data [arg] (list 'quote arg))\n",
        "(defn opaque [] (as-data [UUID (new UUID 0 0)]))\n",
        "(defn stale [] [UUID (UUID. 0 0) (new UUID 0 0)])\n",
    );
    const ALIASES: &str = concat!(
        "(ns demo.aliases (:require [demo.fake :as UUID]) (:import java.util.UUID))\n",
        "(defn probe [x] [UUID (UUID. 0 0) (new UUID 0 0) (instance? UUID x) (UUID/probe)])\n",
    );
    const INVALID: &str =
        "(ns demo.invalid (:import java.util.UUID))\n(defn dangling [] [UUID. java.util.UUID.])\n";
    let live = SOURCE
        .replace("[java.util UUID]", "[java.util ArrayList]")
        .replace("[\"🧪\" UUID", "[\"🧪🦀\" ArrayList")
        .replacen("(instance? UUID x)", "(instance? ArrayList x)", 1)
        .replace("[\"🧪\" (UUID. 0 0)", "[\"🧪🦀\" (ArrayList.)")
        .replacen("(new UUID 0 0)", "(new ArrayList)", 1);
    let root = fixture.0.canonicalize().unwrap();
    let project = root.join("src/demo");
    fs::create_dir_all(&project).unwrap();
    for (name, source) in [
        ("classes.clj", SOURCE),
        ("aliases.clj", ALIASES),
        ("invalid.clj", INVALID),
        ("fake.clj", "(ns demo.fake)\n(defn probe [] :target)\n"),
    ] {
        fs::write(project.join(name), source).unwrap();
    }
    let file = uri(&project.join("classes.clj"));
    let aliases = uri(&project.join("aliases.clj"));
    let invalid = uri(&project.join("invalid.clj"));
    let fake = uri(&project.join("fake.clj"));
    let home = root.join("Synthetic JDK");
    fs::create_dir_all(home.join("jmods")).unwrap();
    fs::create_dir_all(home.join("lib")).unwrap();
    write_archive(
        &home.join("jmods/java.base.jmod"),
        ["java/util/UUID", "java/util/ArrayList", "java/lang/String"]
            .map(|name| (format!("classes/{name}.class"), class_file(name, true))),
    );
    let src = home.join("lib/src.zip");
    write_archive(
        &src,
        [
            ("java.base/java/util/UUID.java", UUID),
            ("java.base/java/util/ArrayList.java", ARRAY_LIST),
            ("java.base/java/lang/String.java", STRING),
        ],
    );
    let reference_cache = root.join("reference-cache");
    let mut child = if reference {
        let config = root.join(".lsp");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("config.edn"),
            format!(
                "{{:hover {{:clojuredocs false}} :log-path {:?} :java {{:jdk-source-uri {:?} :download-jdk-source? false}}}}",
                root.join("reference.log"),
                src,
            ),
        )
        .unwrap();
        let binary =
            std::env::var_os("PARENOX_REFERENCE_LSP").unwrap_or_else(|| "clojure-lsp".into());
        Command::new(binary)
            .arg("listen")
            .current_dir(&root)
            .env("XDG_CACHE_HOME", &reference_cache)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap()
    } else {
        java_lsp(&root, &home, None)
    };
    let source_uri = |resource: &str| {
        if reference {
            uri(&reference_cache
                .join("clojure-lsp/jdk/java.base")
                .join(resource))
        } else {
            format!("jar:{}!/java.base/{resource}", uri(&src))
        }
    };
    let uuid_uri = source_uri("java/util/UUID.java");
    let list_uri = source_uri("java/util/ArrayList.java");
    let string_uri = source_uri("java/lang/String.java");
    let uuid = definition_location(&uuid_uri, 0, 0, 0);
    let list = definition_location(&list_uri, 0, 0, 0);
    let string = definition_location(&string_uri, 0, 0, 0);
    let local = definition_location(&file, 4, 14, 18);
    let var = definition_location(&fake, 1, 6, 11);
    let null = Value::Null;
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut next_id = 0;
    let mut exchange = |mut request: Value| {
        if request.get("id").is_none() {
            send_message(&mut input, &request);
            return Value::Null;
        }
        next_id += 1;
        request["id"] = json!(next_id);
        send_message(&mut input, &request);
        loop {
            let reply = read_message(&mut output).expect("LSP response");
            if reply["id"] == next_id && reply["method"].is_null() {
                assert!(reply["error"].is_null(), "{request} → {reply}");
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
    let capabilities = exchange(
        json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
            "rootUri":uri(&root),"initializationOptions":{"dependencyScheme":"jar"},
            "capabilities":{"textDocument":{"definition":{"linkSupport":true}}}
        }}),
    );
    assert_eq!(capabilities["capabilities"]["definitionProvider"], true);
    exchange(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
    let open = |file: &str, source: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":file,"version":1,"languageId":"clojure","text":source
        }}})
    };
    exchange(open(&file, SOURCE));
    exchange(open(&aliases, ALIASES));
    exchange(open(&invalid, INVALID));
    let check = |exchange: &mut dyn FnMut(Value) -> Value,
                 file: &str,
                 source: &str,
                 query,
                 expected: &Value,
                 retry: bool| {
        let request = java_definition_request(0, file, source, query);
        let mut actual = exchange(request.clone());
        if reference && retry {
            // The reference extracts JDK sources and indexes buffers in the background.
            for _ in 0..60 {
                if actual == *expected {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
                actual = exchange(request.clone());
            }
        }
        assert_eq!(&actual, expected, "{file} at {query:?}");
    };
    for (line, needle, delta, expected) in [
        (1, "UUID", 2, &uuid),
        (2, "UUID", 0, &uuid),
        (2, "UUID", 3, &uuid),
        (2, "java.util.UUID", 2, &uuid),
        (2, "java.util.UUID", 12, &uuid),
        (2, "instance? UUID", 11, &uuid),
        (2, "instance? java.util.UUID", 22, &uuid),
        (2, "String", 2, &string),
        (3, "UUID.", 0, &uuid),
        (3, "UUID.", 4, &uuid),
        (3, "java.util.UUID.", 2, &uuid),
        (3, "java.util.UUID.", 14, &uuid),
        (3, "new UUID", 1, &null),
        (3, "new UUID", 4, &uuid),
        (3, "new java.util.UUID", 15, &uuid),
        (4, "[UUID (", 2, &local),
        (4, "UUID.", 2, if reference { &local } else { &uuid }),
        (4, "new UUID", 5, if reference { &local } else { &uuid }),
        (4, "instance? UUID", 11, &local),
        (5, "'UUID", 2, &null),
        (5, "'java.util.UUID", 12, &null),
        (5, "'(UUID.", 3, &null),
        (5, "quote UUID", 7, &null),
        (5, "new UUID", 5, &null),
        (7, "[UUID", 2, if reference { &uuid } else { &null }),
        (7, "new UUID", 5, if reference { &uuid } else { &null }),
    ] {
        check(
            &mut exchange,
            &file,
            SOURCE,
            (line, needle, delta),
            expected,
            line == 1,
        );
    }
    for (needle, delta, expected) in [
        ("[UUID", 2, &uuid),
        ("UUID.", 4, &uuid),
        ("new UUID", 5, &uuid),
        ("instance? UUID", 11, &uuid),
        ("UUID/probe", 6, &var),
    ] {
        check(
            &mut exchange,
            &aliases,
            ALIASES,
            (1, needle, delta),
            expected,
            false,
        );
    }
    for (needle, delta) in [("UUID.", 2), ("java.util.UUID.", 12)] {
        check(
            &mut exchange,
            &invalid,
            INVALID,
            (1, needle, delta),
            if reference { &uuid } else { &null },
            false,
        );
    }
    for (target, contents) in [(&uuid_uri, UUID), (&string_uri, STRING)] {
        assert_eq!(exchange(read_contents(0, target)), contents);
    }
    exchange(
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":7},"contentChanges":[{"text":live}]
        }}),
    );
    for (line, needle, delta, expected) in [
        (1, "ArrayList", 2, &list),
        (2, "ArrayList", 2, &list),
        (2, "java.util.UUID", 12, &uuid),
        (2, "instance? ArrayList", 12, &list),
        (3, "ArrayList.", 9, &list),
        (3, "java.util.UUID.", 14, &uuid),
        (3, "new ArrayList", 7, &list),
    ] {
        check(
            &mut exchange,
            &file,
            &live,
            (line, needle, delta),
            expected,
            line == 1,
        );
    }
    if !reference {
        for (needle, delta) in [("[UUID", 2), ("UUID.", 2), ("new UUID", 5)] {
            check(
                &mut exchange,
                &file,
                &live,
                (8, needle, delta),
                &null,
                false,
            );
        }
    }
    assert_eq!(exchange(read_contents(0, &list_uri)), ARRAY_LIST);
    assert_eq!(
        fs::read_to_string(project.join("classes.clj")).unwrap(),
        SOURCE
    );
    exchange(json!({"jsonrpc":"2.0","id":0,"method":"shutdown"}));
    exchange(json!({"jsonrpc":"2.0","method":"exit"}));
    drop(input);
    assert!(child.wait().unwrap().success());
}

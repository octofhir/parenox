use super::*;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

#[test]
fn completes_static_java_members_from_jars_and_class_directories() {
    let fixture = Fixture::new("java completion");
    let class_path = "fixture/jvm/Widget.class";
    let bytecode = widget_class("fixture/jvm/Widget", "count", true);
    let jar = fixture.0.join("widget.jar");
    let mut archive = ZipWriter::new(fs::File::create(&jar).unwrap());
    archive
        .start_file(
            class_path,
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .unwrap();
    archive.write_all(&bytecode).unwrap();
    archive.finish().unwrap();
    let class_dir = fixture.0.join("classes");
    let class_file = class_dir.join(class_path);
    fs::create_dir_all(class_file.parent().unwrap()).unwrap();
    fs::write(
        &class_file,
        widget_class("fixture/jvm/Widget", "index", true),
    )
    .unwrap();

    let file = uri(&fixture.0.join("src/demo.clj"));
    let source = "(ns demo (:import (fixture.jvm Widget)))\n\
                   (defn f [] (do \"🦊\" Widget/placeholder))\n\
                   (defn g [] fixture.jvm.Widget/v)\n\
                   (defn h [] (Widget/.incomplete nil))\n\
                   (defn i [] Widget/.visit)\n\
                   (defn j [] Widget/.secret)";
    let row = source.lines().nth(1).unwrap();
    let start = row[..row.find("Widget/").unwrap()].encode_utf16().count();
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":file,"version":1,"languageId":"clojure","text":source
        }}}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/completion","params":{
            "textDocument":{"uri":file},"position":{"line":1,"character":start + "Widget/".len()}
        }}),
        json!({"jsonrpc":"2.0","id":3,"method":"textDocument/completion","params":{
            "textDocument":{"uri":file},"position":{
                "line":2,"character":source.lines().nth(2).unwrap().len() - 1
            }
        }}),
        json!({"jsonrpc":"2.0","id":4,"method":"textDocument/completion","params":{
            "textDocument":{"uri":file},"position":{"line":3,"character":22}
        }}),
        json!({"jsonrpc":"2.0","id":5,"method":"textDocument/completion","params":{
            "textDocument":{"uri":file},"position":{"line":4,"character":24}
        }}),
        json!({"jsonrpc":"2.0","id":6,"method":"textDocument/completion","params":{
            "textDocument":{"uri":file},"position":{"line":5,"character":25}
        }}),
        json!({"jsonrpc":"2.0","id":7,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ];
    let classpath = fixture.0.join("classpath.txt");
    for (first, second, field) in [(&jar, &class_dir, "count"), (&class_dir, &jar, "index")] {
        fs::write(
            &classpath,
            std::env::join_paths([Path::new("src"), first, second])
                .unwrap()
                .as_encoded_bytes(),
        )
        .unwrap();
        let server = lsp_server_args(
            Path::new(env!("CARGO_BIN_EXE_parenox")),
            ["lsp", "--classpath", classpath.to_str().unwrap()],
        );
        let responses = transcript_with(server, &requests);
        let initialize = responses.iter().find(|message| message["id"] == 1).unwrap();
        let triggers =
            initialize["result"]["capabilities"]["completionProvider"]["triggerCharacters"]
                .as_array()
                .unwrap();
        assert!(triggers.contains(&json!(".")));
        let items = |id| {
            responses
                .iter()
                .find(|message| message["id"] == id)
                .unwrap()["result"]["items"]
                .as_array()
                .unwrap()
        };
        let labels: Vec<_> = items(2)
            .iter()
            .map(|item| (&item["label"], &item["kind"]))
            .collect();
        assert_eq!(
            labels,
            [
                (&json!("Widget/VALUE"), &json!(21)),
                (&json!(format!("Widget/{field}")), &json!(5)),
                (&json!("Widget/visit"), &json!(2)),
                (&json!("Widget/visit"), &json!(2))
            ]
        );
        assert_eq!(
            items(2)[2]["textEdit"],
            json!({"newText":"Widget/visit","range":{
                "start":{"line":1,"character":start},
                "end":{"line":1,"character":start + "Widget/placeholder".len()}
            }})
        );
        assert_eq!(items(3).len(), 2);
        assert!(
            items(3)
                .iter()
                .all(|item| item["label"] == "fixture.jvm.Widget/visit")
        );
        assert_eq!(items(3)[0]["detail"], "void ()");
        assert_eq!(items(3)[1]["detail"], "void (int)");
        assert_eq!(items(4).len(), 1);
        assert_eq!(items(4)[0]["label"], "Widget/.instanceCall");
        assert_eq!(items(4)[0]["kind"], 2);
        assert_eq!(items(4)[0]["detail"], "void ()");
        assert_eq!(
            items(4)[0]["textEdit"],
            json!({
                "newText":"Widget/.instanceCall",
                "range":{"start":{"line":3,"character":12},"end":{"line":3,"character":30}}
            })
        );
        assert!(items(5).is_empty(), "static method in instance context");
        assert!(items(6).is_empty(), "private method in instance context");
    }
}

#[test]
fn completes_java_import_tokens_in_saved_and_unfinished_source() {
    let fixture = Fixture::new("Java import completion");
    let jar = fixture.0.join("widget.jar");
    let mut archive = ZipWriter::new(fs::File::create(&jar).unwrap());
    for (name, public) in [
        ("Widget", true),
        ("WidgetHidden", false),
        ("sub/WidgetChild", true),
    ] {
        let internal = format!("fixture/jvm/{name}");
        archive
            .start_file(format!("{internal}.class"), SimpleFileOptions::default())
            .unwrap();
        archive
            .write_all(&widget_class(&internal, "count", public))
            .unwrap();
    }
    archive.finish().unwrap();
    let classes = fixture.0.join("classes/fixture/jvm");
    fs::create_dir_all(&classes).unwrap();
    fs::write(
        classes.join("WidgetFactory.class"),
        widget_class("fixture/jvm/WidgetFactory", "count", true),
    )
    .unwrap();
    fs::write(
        classes.join("WidgetAlias.class"),
        widget_class("fixture/jvm/Other", "count", true),
    )
    .unwrap();
    let classpath = fixture.0.join("classpath.txt");
    fs::write(
        &classpath,
        std::env::join_paths([Path::new("src"), jar.as_path(), Path::new("classes")])
            .unwrap()
            .as_encoded_bytes(),
    )
    .unwrap();

    let path = fixture.0.join("src/demo.clj");
    let file = uri(&path);
    let saved = "(ns demo \"🦊\" (:import fixture.jvm.WidOld [fixture.jvm WidOld]))";
    let overlay = "(ns demo \"🦊\" (:import (fixture.jvm WidOld";
    fs::write(&path, saved).unwrap();
    let complete = |id, text: &str, start: usize| {
        let cursor = text[..start].encode_utf16().count() + "Wid".len();
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/completion","params":{
            "textDocument":{"uri":file},"position":{"line":0,"character":cursor}
        }})
    };
    let open = |source| {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":file,"version":1,"languageId":"clojure","text":source}
        }})
    };
    let server = lsp_server_args(
        Path::new(env!("CARGO_BIN_EXE_parenox")),
        ["lsp", "--classpath", classpath.to_str().unwrap()],
    );
    let messages = transcript_with(
        server,
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
            open(saved),
            complete(2, saved, saved.find("WidOld").unwrap()),
            complete(3, saved, saved.rfind("WidOld").unwrap()),
            json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
                "textDocument":{"uri":file,"version":2},"contentChanges":[{"text":overlay}]
            }}),
            complete(4, overlay, overlay.find("WidOld").unwrap()),
            json!({"jsonrpc":"2.0","id":5,"method":"shutdown"}),
            json!({"jsonrpc":"2.0","method":"exit"}),
        ],
    );
    for (id, text, token, qualified) in [
        (2, saved, "fixture.jvm.WidOld", true),
        (3, saved, "WidOld", false),
        (4, overlay, "WidOld", false),
    ] {
        let result = &messages.iter().find(|message| message["id"] == id).unwrap()["result"];
        let items = result["items"]
            .as_array()
            .unwrap_or_else(|| panic!("request {id}: {result}"));
        assert_eq!(items.len(), 2, "{items:?}");
        let start = text[..text.rfind(token).unwrap()].encode_utf16().count();
        for (item, class) in items.iter().zip(["Widget", "WidgetFactory"]) {
            let full = format!("fixture.jvm.{class}");
            let label = if qualified { full.as_str() } else { class };
            assert_eq!(item["label"], label);
            assert_eq!(item["kind"], 7);
            assert_eq!(item["detail"], full);
            assert_eq!(
                item["textEdit"],
                json!({
                    "newText":label,"range":{
                        "start":{"line":0,"character":start},
                        "end":{"line":0,"character":start + token.len()}
                    }
                })
            );
        }
    }
    assert_eq!(fs::read_to_string(path).unwrap(), saved);
}

fn widget_class(name: &str, field: &str, public: bool) -> Vec<u8> {
    // The native methods keep this small class file independent of javac.
    let mut bytes = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 52];
    let strings = [
        name,
        "java/lang/Object",
        "VALUE",
        field,
        "I",
        "visit",
        "()V",
        "(I)V",
        "instanceCall",
        "secret",
    ];
    bytes.extend_from_slice(&13_u16.to_be_bytes());
    for text in strings {
        bytes.push(1);
        bytes.extend_from_slice(&(text.len() as u16).to_be_bytes());
        bytes.extend_from_slice(text.as_bytes());
    }
    bytes.extend_from_slice(&[7, 0, 1, 7, 0, 2]);
    let class = [
        if public { 0x0021 } else { 0x0020 },
        11,
        12,
        0, // superclass-aware class, no interfaces
        2,
        0x0019,
        3,
        5,
        0,
        0x0009,
        4,
        5,
        0, // fields
        4,
        0x0109,
        6,
        7,
        0,
        0x0109,
        6,
        8,
        0,
        0x0101,
        9,
        7,
        0,
        0x010a,
        10,
        7,
        0, // methods
        0, // class attributes
    ];
    bytes.extend(class.into_iter().flat_map(u16::to_be_bytes));
    bytes
}

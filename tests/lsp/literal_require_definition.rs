use super::*;

fn providers(fixture: &Fixture) {
    let lib = fixture.0.join("src/lab");
    fs::create_dir(&lib).unwrap();
    fs::write(
        lib.join("provider.clj"),
        ";; UTF-16 in a namespace declaration\n(ns ^{:doc \"🧪\"} lab.provider)\n(defn go [x] x)\n",
    )
    .unwrap();
    fs::write(
        lib.join("other.clj"),
        "(ns lab.other)\n(defn go [x] [:other x])\n",
    )
    .unwrap();
    fs::write(lib.join("only.clj"), "(ns lab.only)\n").unwrap();
    fs::write(
        fixture.0.join("src/p.clj"),
        "(ns p)\n(defn go [x] [:namespace-p x])\n",
    )
    .unwrap();
}

fn definition(
    id: usize,
    file: &str,
    source: &str,
    line: usize,
    word: &str,
    offset: usize,
) -> Value {
    let row = source.lines().nth(line).unwrap();
    let character = row[..row.find(word).unwrap()].encode_utf16().count() + offset;
    json!({"jsonrpc":"2.0","id":id,"method":"textDocument/definition","params":{
        "textDocument":{"uri":file},"position":{"line":line,"character":character}
    }})
}

fn location(path: &Path, line: usize, start: usize, end: usize) -> Value {
    json!({"uri":uri(path),"range":{
        "start":{"line":line,"character":start},"end":{"line":line,"character":end}
    }})
}

fn result(replies: &[Value], id: usize) -> &Value {
    let response = replies.iter().find(|message| message["id"] == id).unwrap();
    assert!(response["error"].is_null(), "{response}");
    &response["result"]
}

#[test]
fn literal_require_definition_tracks_statement_order_and_unsaved_bindings() {
    let fixture = Fixture::new("literal require definition");
    providers(&fixture);
    let path = |relative: &str| fixture.0.join("src").join(relative);
    let caller = path("caller.clj");
    let file = uri(&caller);
    let saved = concat!(
        "(ns caller)\n",
        "[(q/go :before) (p/go :namespace-before-alias)]\n",
        "(go :before)\n",
        "(lab.provider/go :before)\n",
        "(require 'lab.only)\n",
        "(clojure.core/require (quote [lab.provider :as p :refer [go]]))\n",
        "(require '[lab.provider :as q])\n",
        "(p/go :aliased)\n",
        "(q/go :after)\n",
        "(go :after)\n",
        "(defn after [x] [\"🧪\" (p/go x) (go x)])\n",
        "(defn shadow [go p] [(go :local) (p/go :aliased)])\n",
        "(require '[lab.provider :as-alias provider-id])\n",
        "(provider-id/go :as-alias)\n",
    );
    let live = saved.replace(
        "[lab.provider :as p :refer [go]]",
        "[lab.other :as p :refer [go]]",
    );
    fs::write(&caller, saved).unwrap();
    let saved_query = |id, line, word, offset| definition(id, &file, saved, line, word, offset);
    let live_query = |id, line, word, offset| definition(id, &file, &live, line, word, offset);
    let replies = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":file,"version":1,"languageId":"clojure","text":saved
        }}}),
        saved_query(2, 1, "q/go", 3),
        saved_query(3, 2, "go", 1),
        saved_query(4, 3, "lab.provider/go", 14),
        saved_query(5, 4, "lab.only", 3),
        saved_query(6, 5, "lab.provider", 4),
        saved_query(7, 5, ":as p", 4),
        saved_query(8, 5, "[go]", 1),
        saved_query(9, 7, "p/go", 3),
        saved_query(10, 8, "q/go", 3),
        saved_query(11, 9, "go", 1),
        saved_query(12, 10, "p/go", 3),
        saved_query(13, 10, "(go", 2),
        saved_query(14, 11, "(go :local", 2),
        saved_query(15, 11, "p/go", 3),
        saved_query(16, 12, "lab.provider", 4),
        saved_query(17, 12, "provider-id", 3),
        saved_query(18, 13, "provider-id/go", 13),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":file,"version":7},"contentChanges":[{"text":live}]
        }}),
        live_query(19, 5, "lab.other", 4),
        live_query(20, 5, ":as p", 4),
        live_query(21, 5, "[go]", 1),
        live_query(22, 10, "p/go", 3),
        live_query(23, 10, "(go", 2),
        live_query(24, 8, "q/go", 3),
        live_query(25, 1, "q/go", 3),
        saved_query(26, 1, "p/go", 3),
        json!({"jsonrpc":"2.0","id":27,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    for (ids, relative, line, start, end) in [
        (
            &[4, 8, 9, 10, 11, 12, 13, 15, 18, 24][..],
            "lab/provider.clj",
            2,
            6,
            8,
        ),
        (&[5], "lab/only.clj", 0, 4, 12),
        (&[6, 7, 16, 17], "lab/provider.clj", 1, 17, 29),
        (&[14], "caller.clj", 11, 14, 16),
        (&[19, 20], "lab/other.clj", 0, 4, 13),
        (&[21, 22, 23], "lab/other.clj", 1, 6, 8),
        (&[26], "p.clj", 1, 6, 8),
    ] {
        let expected = location(&path(relative), line, start, end);
        for id in ids {
            assert_eq!(result(&replies, *id), &expected, "request {id}");
        }
    }
    for id in [2, 3, 25] {
        assert!(
            result(&replies, id).is_null(),
            "request {id}: {}",
            result(&replies, id)
        );
    }
    assert_eq!(fs::read_to_string(caller).unwrap(), saved);
}

#[test]
fn literal_require_definition_resolves_namespace_and_vars_in_selected_jar() {
    use zip::{ZipWriter, write::SimpleFileOptions};

    let fixture = Fixture::new("literal require archive");
    let root = fixture.0.canonicalize().unwrap();
    let archive = root.join("dependency.jar");
    let archived_source = "(ns lab.archived)\n(defn area [x] x)\n";
    let mut archive_file = ZipWriter::new(fs::File::create(&archive).unwrap());
    archive_file
        .start_file("lab/archived.clj", SimpleFileOptions::default())
        .unwrap();
    archive_file.write_all(archived_source.as_bytes()).unwrap();
    archive_file.finish().unwrap();
    let classpath = root.join("classpath.txt");
    fs::write(
        &classpath,
        std::env::join_paths([Path::new("src"), &archive])
            .unwrap()
            .as_encoded_bytes(),
    )
    .unwrap();
    let caller = root.join("src/caller.clj");
    let file = uri(&caller);
    let source = concat!(
        "(ns caller)\n",
        "(arch/area :before)\n",
        "(require '[lab.archived :as arch :refer [area]])\n",
        "(arch/area :after)\n",
        "(area :after)\n",
    );
    fs::write(&caller, source).unwrap();
    let target = format!("jar:{}!/lab/archived.clj", uri(&archive));
    let query = |id, line, word, offset| definition(id, &file, source, line, word, offset);
    let replies = transcript_with(
        lsp_server_args(
            Path::new(env!("CARGO_BIN_EXE_parenox")),
            ["lsp", "--classpath", classpath.to_str().unwrap()],
        ),
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&root)}}),
            json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
                "uri":file,"version":1,"languageId":"clojure","text":source
            }}}),
            query(2, 1, "arch/area", 6),
            query(3, 2, "lab.archived", 4),
            query(4, 2, ":as arch", 6),
            query(5, 2, "[area]", 3),
            query(6, 3, "arch/area", 6),
            query(7, 4, "area", 2),
            json!({"jsonrpc":"2.0","id":8,"method":"clojure/dependencyContents","params":{"uri":target}}),
            json!({"jsonrpc":"2.0","id":9,"method":"shutdown"}),
            json!({"jsonrpc":"2.0","method":"exit"}),
        ],
    );
    assert!(result(&replies, 2).is_null());
    for (ids, line, start, end) in [(&[3, 4][..], 0, 4, 16), (&[5, 6, 7], 1, 6, 10)] {
        let expected = json!({"uri":target,"range":{
            "start":{"line":line,"character":start},"end":{"line":line,"character":end}
        }});
        for id in ids {
            assert_eq!(result(&replies, *id), &expected, "request {id}");
        }
    }
    assert_eq!(result(&replies, 8), archived_source);
}

#[test]
fn literal_require_definition_keeps_first_alias_and_ignores_unproved_calls() {
    let fixture = Fixture::new("literal require conflicts and guards");
    providers(&fixture);
    let path = |relative: &str| fixture.0.join("src").join(relative);
    let caller = path("caller.clj");
    let file = uri(&caller);
    let source = concat!(
        "(ns caller (:require [lab.other :as p]))\n",
        "(p/go :before)\n",
        "(require '[lab.provider :as p])\n",
        "(p/go :first-alias)\n",
        "(require '[lab.provider :as q :refer [go]])\n",
        "(go :first-referral)\n",
        "(require '[lab.other :as q])\n",
        "(q/go :first-alias)\n",
        "(require '[lab.other :refer [go]])\n",
        "(go :latest-referral)\n",
        "(let [require identity]\n",
        "  (require '[lab.provider :as inside]))\n",
        "(inside/go :not-loaded)\n",
        "(comment (require '[lab.provider :as commented]))\n",
        "(commented/go :not-loaded)\n",
        "'(require '[lab.provider :as quoted])\n",
        "(quoted/go :not-loaded)\n",
        "(defn require [spec] spec)\n",
        "(require '[lab.provider :as fake])\n",
        "(fake/go :shadowed)\n",
        "(clojure.core/require '[lab.provider :as real])\n",
        "(real/go :loaded)\n",
    );
    fs::write(&caller, source).unwrap();
    let query = |id, line, word, offset| definition(id, &file, source, line, word, offset);
    let replies = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":file,"version":1,"languageId":"clojure","text":source
        }}}),
        query(2, 1, "p/go", 3),
        query(3, 3, "p/go", 3),
        query(4, 5, "go", 1),
        query(5, 7, "q/go", 3),
        query(6, 9, "go", 1),
        query(7, 11, "lab.provider", 4),
        query(8, 12, "inside/go", 8),
        query(9, 13, "lab.provider", 4),
        query(10, 14, "commented/go", 11),
        query(11, 15, "lab.provider", 4),
        query(12, 16, "quoted/go", 8),
        query(13, 18, "lab.provider", 4),
        query(14, 19, "fake/go", 6),
        query(15, 20, "lab.provider", 4),
        query(16, 21, "real/go", 6),
        json!({"jsonrpc":"2.0","id":17,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let other = location(&path("lab/other.clj"), 1, 6, 8);
    for id in [2, 3, 6] {
        assert_eq!(result(&replies, id), &other, "request {id}");
    }
    let provider = location(&path("lab/provider.clj"), 2, 6, 8);
    for id in [4, 5, 16] {
        assert_eq!(result(&replies, id), &provider, "request {id}");
    }
    for id in 7..=14 {
        assert!(
            result(&replies, id).is_null(),
            "request {id}: {}",
            result(&replies, id)
        );
    }
    assert_eq!(
        result(&replies, 15),
        &location(&path("lab/provider.clj"), 1, 17, 29)
    );
}

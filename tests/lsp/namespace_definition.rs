use super::*;

#[test]
fn workspace_namespace_definition_uses_the_unique_live_provider() {
    let fixture = Fixture::new("namespace definition");
    let provider = fixture.0.join("src/catalog.cljc");
    let other = fixture.0.join("src/other.clj");
    let caller = fixture.0.join("src/caller.clj");
    let saved = "(ns ^{:doc \"🧪\"} lib.catalog)\n(defn title [x] x)\n";
    let live = "; editing\n(ns ^{:doc \"🦊 changed\"} lib.catalog)\n(defn title [x] x)\n";
    let unrelated = "(ns lib.unrelated)\n";
    let consumer = concat!(
        "(ns caller\n",
        "  (:require lib.catalog\n",
        "            [lib.catalog :as catalog :refer [title]]\n",
        "            [lib.catalog :as-alias catalog-id]))\n",
    );
    fs::write(&provider, saved).unwrap();
    fs::write(&other, unrelated).unwrap();
    fs::write(&caller, consumer).unwrap();

    let query = |id, line, character| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/definition","params":{
            "textDocument":{"uri":uri(&caller)},"position":{"line":line,"character":character}
        }})
    };
    let open = |file: &Path, version, text| {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":uri(file),"version":version,"languageId":"clojure","text":text
        }}})
    };
    let change = |version, text| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri(&provider),"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let location = |line, start, end| {
        json!({"uri":uri(&provider),"range":{
            "start":{"line":line,"character":start},"end":{"line":line,"character":end}
        }})
    };
    let responses = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        open(&caller, 1, consumer),
        query(2, 1, 15),
        query(3, 2, 16),
        query(4, 2, 31),
        query(5, 3, 16),
        query(6, 3, 38),
        query(7, 2, 47),
        open(&provider, 1, saved),
        change(7, live),
        change(6, "(ns stale)"),
        query(8, 1, 15),
        query(9, 2, 31),
        query(10, 3, 38),
        query(11, 2, 47),
        open(&other, 1, "(ns lib.catalog)\n"),
        query(12, 1, 15),
        query(13, 2, 31),
        query(14, 3, 38),
        json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{
            "textDocument":{"uri":uri(&other)}
        }}),
        query(15, 2, 16),
        change(8, "(ns lib.retired)\n(defn title [x] x)\n"),
        query(16, 1, 15),
        query(17, 2, 47),
        json!({"jsonrpc":"2.0","id":18,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| {
        let response = responses
            .iter()
            .find(|message| message["id"] == id)
            .unwrap();
        assert!(response["error"].is_null(), "{response}");
        &response["result"]
    };

    for id in 2..=6 {
        assert_eq!(result(id), &location(0, 17, 28), "request {id}");
    }
    assert_eq!(result(7), &location(1, 6, 11));
    for id in [8, 9, 10, 15] {
        assert_eq!(result(id), &location(1, 25, 36), "request {id}");
    }
    assert_eq!(result(11), &location(2, 6, 11));
    for id in [12, 13, 14, 16, 17] {
        assert!(result(id).is_null(), "request {id}: {}", result(id));
    }
    assert_eq!(fs::read_to_string(provider).unwrap(), saved);
    assert_eq!(fs::read_to_string(other).unwrap(), unrelated);
}

#[test]
fn prefixed_libspec_definitions_follow_the_live_prefix_and_alias() {
    let fixture = Fixture::new("prefixed libspec definition");
    let path = |relative: &str| fixture.0.join("src").join(relative);
    fs::create_dir(path("lib")).unwrap();
    fs::create_dir(path("alt")).unwrap();
    for (relative, source) in [
        ("lib.clj", "(ns lib)\n"),
        ("alt.clj", "(ns alt)\n"),
        ("lib/catalog.clj", "(ns lib.catalog)\n"),
        ("lib/math.clj", "(ns lib.math)\n(defn add [a b] (+ a b))\n"),
        ("lib/text.clj", "(ns lib.text)\n(defn title [] :title)\n"),
        ("lib/tool.clj", "(ns lib.tool)\n(defn lookup [] :found)\n"),
        (
            "alt/math.clj",
            "(ns ^{:doc \"🦊\"} alt.math)\n(defn ^{:doc \"🧪\"} add [a b] (+ a b))\n",
        ),
    ] {
        fs::write(path(relative), source).unwrap();
    }
    let source = concat!(
        "(ns caller\n",
        "  (:require (lib catalog [math :as m :refer [add]] (text :as words)))\n",
        "  (:use [lib (tool :as tools :only [lookup])]))\n",
        "(defn demo []\n",
        "  [\"🧪\" (m/add 1 2) (add 1 2) (words/title) (tools/lookup) (lookup)])\n",
    );
    let updated = source
        .replace("(lib catalog", "(alt catalog")
        .replace(":as m ", ":as n ")
        .replace("(m/add", "(n/add")
        + "(m/add 1 2)\n";
    let caller = path("caller.clj");
    let caller_uri = uri(&caller);
    fs::write(&caller, source).unwrap();

    let query = |id, line, character| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/definition","params":{
            "textDocument":{"uri":caller_uri},"position":{"line":line,"character":character}
        }})
    };
    let responses = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":uri(&fixture.0)}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{
            "uri":caller_uri,"languageId":"clojure","version":1,"text":source
        }}}),
        query(2, 1, 14),
        query(3, 1, 18),
        query(4, 1, 27),
        query(5, 1, 35),
        query(6, 1, 46),
        query(7, 1, 53),
        query(8, 1, 62),
        query(9, 2, 10),
        query(10, 2, 15),
        query(11, 2, 25),
        query(12, 2, 37),
        query(13, 4, 12),
        query(14, 4, 22),
        query(15, 4, 39),
        query(16, 4, 53),
        query(17, 4, 61),
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":caller_uri,"version":7},"contentChanges":[{"text":updated}]
        }}),
        query(18, 1, 14),
        query(19, 1, 18),
        query(20, 1, 27),
        query(21, 1, 35),
        query(22, 1, 46),
        query(23, 4, 12),
        query(24, 4, 22),
        query(25, 5, 4),
        query(26, 4, 53),
        json!({"jsonrpc":"2.0","id":27,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let result = |id| {
        let response = responses
            .iter()
            .find(|message| message["id"] == id)
            .unwrap();
        assert!(response["error"].is_null(), "{response}");
        &response["result"]
    };
    for (ids, relative, line, start, end) in [
        (&[3][..], "lib/catalog.clj", 0, 4, 15),
        (&[4, 5], "lib/math.clj", 0, 4, 12),
        (&[6, 13, 14], "lib/math.clj", 1, 6, 9),
        (&[7, 8], "lib/text.clj", 0, 4, 12),
        (&[15], "lib/text.clj", 1, 6, 11),
        (&[10, 11], "lib/tool.clj", 0, 4, 12),
        (&[12, 16, 17, 26], "lib/tool.clj", 1, 6, 12),
        (&[20, 21], "alt/math.clj", 0, 17, 25),
        (&[22, 23, 24], "alt/math.clj", 1, 19, 22),
    ] {
        let expected = json!({"uri":uri(&path(relative)),"range":{
            "start":{"line":line,"character":start},"end":{"line":line,"character":end}
        }});
        for id in ids {
            assert_eq!(result(*id), &expected, "request {id}");
        }
    }
    for id in [2, 9, 18, 19, 25] {
        assert!(result(id).is_null(), "request {id}: {}", result(id));
    }
    assert_eq!(fs::read_to_string(caller).unwrap(), source);
}

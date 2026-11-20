use std::collections::BTreeSet;

use parenox_analysis::{Session, Source};
use parenox_lsp::completion::{DependencyCompletions, complete};
use parenox_lsp::text::Lines;
use serde_json::{Value, json};

fn complete_from(cursor: &str) -> Option<Value> {
    let offset = cursor.find('|').unwrap();
    let source = cursor.replacen('|', "", 1);
    let mut session = Session::default();
    session.check_project_documents(
        &[
            Source {
                path: "app.clj",
                source: &source,
            },
            Source {
                path: "my/app.clj",
                source: "(ns my.app) (defn hello [] nil) (defn- hello-private [] nil)",
            },
            Source {
                path: "my/application.cljc",
                source: "(ns my.application)",
            },
        ],
        &[],
        &BTreeSet::from(["app.clj"]),
    );
    complete(
        &session,
        "app.clj",
        "file:///app.clj",
        &Lines::new(&source),
        offset,
        true,
        &DependencyCompletions::default(),
    )
}

#[test]
fn completes_closed_namespaces_and_replaces_the_whole_symbol_in_utf16() {
    let result = complete_from("(ns app \"😀\" (:require [my.ap|old :as lib]))").unwrap();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["label"], "my.app");
    assert_eq!(items[1]["label"], "my.application");
    assert_eq!(
        items[0]["textEdit"],
        json!({"newText":"my.app","range":{"start":{"line":0,"character":24},"end":{"line":0,"character":32}}})
    );
    let result = complete_from("(ns app (:require my.app|))").unwrap();
    assert_eq!(result["items"].as_array().unwrap().len(), 2);
}

#[test]
fn completes_public_vars_in_an_unclosed_refer_vector() {
    let result = complete_from("(clojure.core/ns app (:require [my.app :as m :refer [he|").unwrap();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["label"], "hello");
    assert_eq!(items[0]["textEdit"]["newText"], "hello");
}

#[test]
fn skips_inert_and_other_namespace_clause_slots() {
    assert!(complete_from("(ns app (:require #_[my.ap|]))").is_none());
    assert!(complete_from("(ns app (:require ; my.ap|\n))").is_none());
    assert!(complete_from("(ns app (:require [\"my.ap|\"]))").is_none());
    for cursor in [
        "(ns app (:require [my.app :as my.ap|]))",
        "(ns app (:require [my.app :exclude [he|]]))",
        "(comment (ns app (:require [my.ap|])))",
    ] {
        assert!(
            complete_from(cursor).unwrap()["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

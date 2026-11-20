use parenox::{Session, Source};

fn test_move(client: &str) -> Result<Vec<parenox::Definition>, String> {
    let mut session = Session::default();
    session.check_project(
        &[
            Source {
                path: "src/demo/old.clj",
                source: "(ns demo.old) (defn render [x] x)",
            },
            Source {
                path: "src/demo/client.clj",
                source: client,
            },
        ],
        None,
    );
    session.namespace_move("src/demo/old.clj", "demo.old", "demo.new")
}

#[test]
fn namespace_moves_reject_possible_destination_capture_and_unresolved_source_references() {
    let clients = [
        "(ns demo.client (:require [demo.old :as old] [vendor :as demo.new]))
         (defn show [x] (demo.old/render x))",
        "(ns demo.client (:require [demo.old :as old] [demo.new :as other]))
         (defn show [x] (old/render x))",
        "(ns demo.client (:require [demo.old :as old]))
         (defn show [] (requiring-resolve 'demo.old/render))",
    ];
    for client in clients {
        let error = test_move(client).expect_err("an unsafe plan must have no edits");
        assert!(error.contains("demo."), "{error}");
    }
}

#[test]
fn keeps_a_literal_alias_with_the_old_namespaces_spelling() {
    let client = "(ns demo.client (:require [demo.old :as demo.old]))
                  (defn show [x] (demo.old/render x))";
    let edits = test_move(client).unwrap();
    assert_eq!(edits.len(), 2);
    let [edit, _] = edits.as_slice() else {
        unreachable!()
    };
    assert_eq!(edit.path, "src/demo/client.clj");
    assert_eq!(
        edit.span.start.offset,
        client.find("[demo.old").unwrap() + 1
    );
}

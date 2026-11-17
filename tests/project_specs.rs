use parenox::{Session, Source};

const APP: Source<'_> = Source {
    path: "app.clj",
    source: include_str!("../examples/namespace_specs/app.clj"),
};
const CONFIG: Source<'_> = Source {
    path: "config.clj",
    source: include_str!("../examples/namespace_specs/config.clj"),
};
const HANDLER: Source<'_> = Source {
    path: "handler.clj",
    source: include_str!("../examples/namespace_specs/handler.clj"),
};

#[test]
fn alias_only_requires_do_not_create_cycles_and_spec_keys_follow_their_owner() {
    let mut session = Session::default();
    let first = session.check_project(&[APP, CONFIG, HANDLER], None);
    assert!(first.ok && first.complete, "{first:#?}");
    let reordered = session.check_project(&[HANDLER, CONFIG, APP], None);
    assert!(reordered.ok && reordered.complete, "{reordered:#?}");
    assert_eq!(reordered.stats.parsed_files, 0);
    assert_eq!(reordered.stats.checked_functions, 0);
    assert_eq!(reordered.stats.reused_functions, 4);

    let edited = HANDLER
        .source
        .replace("(s/def ::id string?)", "(s/def ::id number?)");
    let changed = session.check_project(
        &[
            APP,
            CONFIG,
            Source {
                source: &edited,
                ..HANDLER
            },
        ],
        None,
    );
    assert!(!changed.ok && changed.complete, "{changed:#?}");
    assert_eq!(changed.stats.parsed_files, 1);
    let [app, config, handler] = changed.files.as_slice() else {
        panic!("{changed:#?}");
    };
    assert!(handler.ok && config.ok);
    assert_eq!(app.diagnostics[0].expected.as_deref(), Some("Number"));
    assert_eq!(app.diagnostics[0].actual.as_deref(), Some("String"));

    let removed = session.check_project(&[APP, CONFIG], None);
    assert!(!removed.ok && !removed.complete, "{removed:#?}");
    assert_eq!(removed.stats.parsed_files, 0);
    for (file, key) in removed
        .files
        .iter()
        .zip([":wire/id", ":demo.namespace-specs.handler/id"])
    {
        assert!(
            file.diagnostics
                .iter()
                .any(|d| d.message == format!("No declaration for spec {key}")),
            "{file:#?}"
        );
    }
    let restored = session.check_project(&[APP, CONFIG, HANDLER], None);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn recursive_and_ambiguous_project_specs_report_the_declaration() {
    let one = Source {
        path: "one.clj",
        source: "(ns one (:require [clojure.spec.alpha :as s]))
                 (s/def :wire/item :wire/other)",
    };
    let two = Source {
        path: "two.clj",
        source: "(ns two (:require [clojure.spec.alpha :as s]))
                 (s/def :wire/other :wire/item)",
    };
    let mut session = Session::default();
    let recursive = session.check_project(&[one, two], None);
    assert!(!recursive.complete);
    assert!(recursive.files[0].diagnostics.is_empty());
    let diagnostic = &recursive.files[1].diagnostics[0];
    assert_eq!(diagnostic.code, "PNX9001");
    assert!(diagnostic.message.contains("Recursive spec"));
    assert_eq!(
        &two.source[diagnostic.span.start.offset..diagnostic.span.end.offset],
        ":wire/item"
    );

    let duplicate = [
        Source {
            source: "(ns one) (clojure.spec.alpha/def :wire/item string?)",
            ..one
        },
        Source {
            source: "(ns two) (clojure.spec.alpha/def :wire/item number?)",
            ..two
        },
    ];
    for input in [duplicate, [duplicate[1], duplicate[0]]] {
        let ambiguous = session.check_project(&input, None);
        assert!(!ambiguous.ok && !ambiguous.complete, "{ambiguous:#?}");
        for file in ambiguous.files {
            assert_eq!(file.diagnostics.len(), 1);
            assert!(
                file.diagnostics[0]
                    .message
                    .contains("declared in more than one source file")
            );
        }
    }
}

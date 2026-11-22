use parenox::{Session, Source};

const SOURCE: &str = include_str!("../examples/namespaced_keys.cljc");

#[test]
fn a_reader_effect_can_give_equal_keyword_spellings_distinct_values() {
    let namespace = "(ns reader.keyword-context (:require [reader.original :as-alias wire]))";
    for literal in [
        "#{::wire/value #_#oracle/rebind nil ::wire/value}",
        "#:probe{::wire/value 1 #_#oracle/rebind nil ::wire/value 2}",
    ] {
        let source = format!("{namespace}\n(def result {literal})");
        let parsed = parenox::reader::parse(&source);
        assert!(parsed.errors.is_empty(), "{parsed:#?}");
        assert_eq!(parsed.syntax().text().to_string(), source);
        assert_eq!(parenox::reader::read(&source).unwrap().len(), 2);

        let report = Session::default().check("keyword-context.clj", &source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, "PNX9001");
        assert_eq!(
            finding.message,
            "Reader namespace is unchecked after a data reader call"
        );
        assert_eq!(
            finding.span.start.offset,
            source.rfind("::wire/value").unwrap()
        );
    }
    let error = parenox::reader::read("#{::wire/value ::wire/value}").unwrap_err();
    assert_eq!(error.message, "Duplicate element in set literal");
}

#[test]
fn aliases_qualify_auto_map_keys_under_their_source_namespace() {
    let mut session = Session::default();
    let good = session.check("namespaced_keys.cljc", SOURCE);
    assert!(good.ok && good.complete, "{good:#?}");
    let edited = SOURCE.replace(":_/source", "::payload/sku");
    let bad = session.check("namespaced_keys.cljc", &edited);
    assert!(!bad.complete, "{bad:#?}");
    let [error] = bad.diagnostics.as_slice() else {
        panic!("{bad:#?}");
    };
    assert_eq!(error.code, "PNX9000");
    assert_eq!(
        &edited[error.span.start.offset..error.span.end.offset],
        "::payload/sku"
    );
}

#[test]
fn namespace_resolution_preserves_contracts_written_as_metadata_maps() {
    let source = r#"
(ns catalog.metadata (:require [t :as-alias contract]))
(defn ^#::contract{:sig '[number? :-> number?]} plus-one [n] (inc n))
(defn ^#::{:t/sig '[:-> number?]} call-it [] (plus-one 1))
"#;
    let mut session = Session::default();
    let path = "catalog/metadata.clj";
    let good = session.check(path, source);
    assert!(good.ok && good.complete, "{good:#?}");

    let bad = source.replace("(plus-one 1)", r#"(plus-one "text")"#);
    let report = session.check(path, &bad);
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(
        &bad[diagnostic.span.start.offset..diagnostic.span.end.offset],
        r#""text""#
    );
}

#[test]
fn resolved_literals_reject_duplicate_keys_and_elements_at_read_time() {
    let namespace = "(ns catalog.metadata (:require [catalog.items :as-alias items]))";
    let path = "catalog/metadata.clj";
    let mut session = Session::default();
    let good = format!("{namespace}\n(def data '#{{::local :other/local}})");
    assert!(session.check(path, &good).complete);

    for (expression, repeated) in [
        (
            "#{::local :catalog.metadata/local}",
            ":catalog.metadata/local",
        ),
        ("{::items/key 1 :catalog.items/key 2}", ":catalog.items/key"),
        (
            "^{::items/key :first :catalog.items/key :second} []",
            ":catalog.items/key",
        ),
        (
            "{[::items/key] 1 [:catalog.items/key] 2}",
            "[:catalog.items/key]",
        ),
        (
            "#{#::items{:key 1} #:catalog.items{:key 1}}",
            "#:catalog.items{:key 1}",
        ),
    ] {
        let source = format!("{namespace}\n(def data '{expression})");
        let report = session.check(path, &source);
        let [error] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert!(!report.complete && !report.ok, "{report:#?}");
        assert_eq!(error.code, "PNX9000");
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            repeated
        );
    }
    let rolled_back = session.check(path, &good);
    assert!(rolled_back.ok && rolled_back.complete, "{rolled_back:#?}");
}

#[test]
fn namespace_changes_after_the_whole_conditional_has_been_read() {
    let path = "reader/boundary.cljc";
    let mut session = Session::default();
    let valid = r#"#?(:clj (ns reader.boundary (:require [catalog.items :as-alias items]))
   :cljs :inactive)
(def item ::items/id)
"#;
    let report = session.check(path, valid);
    assert!(report.ok && report.complete, "{report:#?}");
    let during_read = valid.find(":inactive").unwrap();
    assert!(session.reader_namespace_before(path, during_read).is_none());
    let after_read = valid.find("(def item").unwrap();
    assert_eq!(
        session
            .reader_namespace_before(path, after_read)
            .map(|(name, _)| name),
        Some("reader.boundary")
    );

    for (source, repeated) in [
        (valid.replace(":inactive", "::items/id"), "::items/id"),
        (
            "#?(:clj (ns reader.boundary) :cljs #{::local :user/local})".into(),
            ":user/local",
        ),
    ] {
        let report = session.check(path, &source);
        let [error] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(error.code, "PNX9000");
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            repeated
        );
    }
    let rolled_back = session.check(path, valid);
    assert!(rolled_back.ok && rolled_back.complete, "{rolled_back:#?}");
}

#[test]
fn forms_read_before_ns_keep_their_original_keyword_namespace() {
    let path = "reader/boundary.cljc";
    let source = r#"#?(:clj (clojure.core/defn prior {:t/sig '[:-> {:user/label #{:user/value}}]} []
  {::label ::value}))
#?(:clj (ns reader.boundary) :cljs ::outside)
(defn later {:t/sig '[:-> #{:reader.boundary/label}]} [] ::label)"#;
    let mut session = Session::default();
    let report = session.check(path, source);
    assert!(report.ok && report.complete, "{report:#?}");

    let duplicate = source.replace("{::label ::value}", "{::label ::value :user/label nil}");
    let report = session.check(path, &duplicate);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX9000");
    assert_eq!(
        &duplicate[error.span.start.offset..error.span.end.offset],
        ":user/label"
    );
    let rollback = session.check(path, source);
    assert!(rollback.ok && rollback.complete, "{rollback:#?}");
}

#[test]
fn reader_alias_failures_are_not_hidden_by_later_structural_errors() {
    let mut session = Session::default();
    for (source, first, message) in [
        (
            "(ns reader.errors)\n(def item ::unknown/id)\n(defn unfinished []",
            "::unknown/id",
            "No namespace alias unknown",
        ),
        (
            "(ns reader.errors)\n(def item [::unknown/id",
            "::unknown/id",
            "No namespace alias unknown",
        ),
        ("#::unknown{:item", "unknown", "No namespace alias unknown"),
        (
            "#?(:clj (ns reader.errors (:require [catalog :as-alias items]))\n   :cljs ::items/id)\n(defn unfinished []",
            "::items/id",
            "No namespace alias items",
        ),
        ("^nil ::unknown/id", "^nil", "Metadata requires"),
        (
            "#::unknown",
            "#::unknown",
            "Expected a map after the namespace",
        ),
        (
            "(ns reader.errors (:require [catalog :as-alias items]))\n(def item ::items/id)\n)\n(ns other)\n::unknown/id",
            ")",
            "Unexpected closing delimiter",
        ),
    ] {
        let report = session.check("reader/errors.cljc", source);
        let [error] = report.diagnostics.as_slice() else {
            panic!("{source}: {report:#?}");
        };
        assert_eq!(error.code, "PNX9000");
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            first
        );
        assert!(error.message.contains(message), "{source}: {report:#?}");
    }
}

#[test]
fn resolved_literal_errors_follow_read_order() {
    let path = "reader/errors.cljc";
    let mut session = Session::default();
    for (body, offending) in [
        (
            "(def values #{::key :reader.errors/key}) ::missing/x",
            ":reader.errors/key",
        ),
        (
            "(def values #{::key :reader.errors/key ::missing/x})",
            "::missing/x",
        ),
        (
            "{:nested #{::key :reader.errors/key} :later ::missing/x}",
            ":reader.errors/key",
        ),
        (
            "#::{:outer 1 :reader.errors/outer #{::inner :reader.errors/inner}}",
            ":reader.errors/inner",
        ),
        (
            "#_#{::key :reader.errors/key} ::missing/x",
            ":reader.errors/key",
        ),
        (
            "#?(:cljs #{::key :reader.errors/key} :clj nil) ::missing/x",
            ":reader.errors/key",
        ),
        (
            "(def values #{::outer :reader.errors/outer\n #_#{::inner :reader.errors/inner}})",
            ":reader.errors/inner",
        ),
        (
            "(def values #{::key :reader.errors/key}) (defn unfinished []",
            ":reader.errors/key",
        ),
    ] {
        let source = format!("(ns reader.errors)\n{body}");
        let report = session.check(path, &source);
        let [error] = report.diagnostics.as_slice() else {
            panic!("{body}: {report:#?}");
        };
        assert_eq!(error.code, "PNX9000");
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            offending,
            "{body}"
        );
    }
}

#[test]
fn damaged_parents_keep_completed_children_and_the_original_error_boundary() {
    for (source, offending, message) in [
        (
            "(def data [) ::missing/k])",
            ")",
            "Unexpected closing delimiter",
        ),
        (
            "(def data [#{::k :reader.errors/k} )])",
            ":reader.errors/k",
            "Duplicate element",
        ),
        (
            "(def data {:key #{::k :reader.errors/k} :odd})",
            ":reader.errors/k",
            "Duplicate element",
        ),
        (
            "(def data #{::k :reader.errors/k 1 1})",
            ":reader.errors/k",
            "Duplicate element",
        ),
        (
            "(def data '{~::k :first (clojure.core/unquote :reader.errors/k) :second})",
            "(clojure.core/unquote :reader.errors/k)",
            "Duplicate key",
        ),
        (
            "(def data #?(:clj #{::k :reader.errors/k} :cljs ]))",
            ":reader.errors/k",
            "Duplicate element",
        ),
    ] {
        let source = format!("(ns reader.errors)\n{source}");
        let report = Session::default().check("reader/errors.cljc", &source);
        let [error] = report.diagnostics.as_slice() else {
            panic!("{source}: {report:#?}");
        };
        assert_eq!(error.code, "PNX9000");
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            offending,
            "{source}"
        );
        assert!(error.message.contains(message), "{source}: {report:#?}");
    }
}

#[test]
fn reading_multiple_namespaces_uses_aliases_from_the_active_namespace() {
    let path = "reader/namespaces.cljc";
    let source = r#"#?(:clj (ns reader.first (:require [library.first :as-alias tag]))
   :cljs #{::local :reader.first/local})
(def before ::tag/item)
#?(:clj (ns reader.second (:require [library.second :as-alias tag]))
   :cljs ::tag/still-first)
(def after #::tag{:item 1})
#?(:clj (ns reader.first)
   :cljs #{::tag/inside :library.first/inside})
(def restored ::tag/item)
"#;
    let report = Session::default().check(path, source);
    assert!(!report.complete);
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    assert!(
        report.diagnostics.iter().all(|finding| {
            finding.code == "PNX9001" && finding.message == "Only one ns declaration is supported"
        }),
        "{report:#?}"
    );

    let broken = r#"(ns reader.first (:require [library.first :as-alias tag]))
(def before ::tag/item)
(ns reader.second)
(def after ::tag/item)
"#;
    let report = Session::default().check(path, broken);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9000");
    assert_eq!(finding.span.start.line, 4);
    assert_eq!(
        &broken[finding.span.start.offset..finding.span.end.offset],
        "::tag/item"
    );

    let forward = r#"(ns reader.first)
(def before ::late/item)
(ns reader.first (:require [library.first :as-alias late]))
(def after ::late/item)
"#;
    let report = Session::default().check(path, forward);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9000");
    assert_eq!(finding.span.start.line, 2);
    assert_eq!(
        &forward[finding.span.start.offset..finding.span.end.offset],
        "::late/item"
    );
}

#[test]
fn a_statically_known_require_prefix_provides_read_time_aliases() {
    let path = "reader/literal_require.cljc";
    let source = r#"(ns reader.require (:require [clojure.string]))
(def stamp {:revision 1})
(comment (anything/only-read))
(require '[clojure.string :as text] '[library.items :as-alias items])
(def mark ::items/id)
(def row #::items{:sku "ready"})
(defn text-tag {:t/sig '[:-> #{:clojure.string/tag}]} [] ::text/tag)
"#;
    let mut session = Session::default();
    let report = session.check(path, source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
    assert!(finding.message.contains("require"));

    let duplicate = source.replace(
        "(def mark ::items/id)",
        "(def mark #{::items/id :library.items/id})",
    );
    let report = session.check(path, &duplicate);
    let duplicate_key = report.diagnostics[0].span;
    assert_eq!(report.diagnostics[0].code, "PNX9000", "{report:#?}");
    assert_eq!(
        &duplicate[duplicate_key.start.offset..duplicate_key.end.offset],
        ":library.items/id"
    );

    let unload = source.replace("(defn text-tag", "(ns-unalias *ns* 'text)\n(defn text-tag");
    let report = session.check(path, &unload);
    assert_eq!(report.diagnostics[0].code, "PNX9001", "{report:#?}");
    assert!(report.diagnostics[0].message.contains("top-level effect"));
    let alias = report.diagnostics[0].span;
    assert_eq!(&unload[alias.start.offset..alias.end.offset], "::text/tag");

    let same_read = r#"(ns reader.require)
#?(:clj (require '[library.items :as-alias items]) :cljs ::items/id)"#;
    let report = session.check(path, same_read);
    assert_eq!(report.diagnostics[0].code, "PNX9000", "{report:#?}");

    let shadowed = r#"(ns reader.require (:refer-clojure :exclude [require]))
(require '[library.items :as-alias items])
(def mark ::items/id)"#;
    let report = session.check(path, shadowed);
    assert_eq!(report.diagnostics[0].code, "PNX9000", "{report:#?}");

    let shadowed = source.replace("(def stamp {:revision 1})", "(def require nil)");
    let report = session.check(path, &shadowed);
    assert_eq!(report.diagnostics[0].code, "PNX9000", "{report:#?}");
    let qualified = shadowed.replace("(require '", "(clojure.core/require '");
    let report = session.check(path, &qualified);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
    assert!(finding.message.contains("clojure.core/require"));

    let restored = session.check(path, source);
    assert_eq!(restored.diagnostics.len(), 1, "{restored:#?}");
    assert_eq!(restored.diagnostics[0].code, "PNX9001");
}

#[test]
fn alias_uses_after_a_possible_library_load_report_missing_context() {
    let source =
        "(ns reader.loading)\n(require '[clojure.string :as text])\n(def mark ::text/mark)";
    let path = "reader/loading.cljc";
    let report = Session::default().check(path, source);
    assert!(!report.complete, "{report:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
    assert!(finding.message.contains("may load a library"));
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "::text/mark"
    );

    let same_read =
        "(ns reader.loading)\n#?(:clj (require '[clojure.string :as text]) :cljs ::text/mark)";
    let report = Session::default().check(path, same_read);
    assert_eq!(report.diagnostics[0].code, "PNX9000", "{report:#?}");
}

#[test]
fn a_data_reader_can_change_the_namespace_after_reading_its_payload() {
    let path = "reader/tagged_scope.cljc";
    let mut session = Session::default();
    for (source, spelling) in [
        (
            "(ns reader.original)\n#_#test/change :payload\n(def saved ::wire/item)",
            "::wire/item",
        ),
        (
            "(ns reader.original (:require [reader.prior :as-alias wire]))\n\
             #_#test/change ::wire/payload\n(def saved ::wire/item)",
            "::wire/item",
        ),
        (
            "(ns reader.original)\n#_#test/change :payload\n(def saved ::local)",
            "::local",
        ),
        (
            "(ns reader.original)\n#_#test/change :payload\n(def saved #::wire{:x 1})",
            "wire",
        ),
        (
            "(ns reader.original)\n#_#test/outer [#test/change :payload\n::wire/item]",
            "::wire/item",
        ),
    ] {
        let report = session.check(path, source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, "PNX9001");
        assert!(finding.message.contains("after a data reader"));
        assert_eq!(finding.span.start.line, 3);
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            spelling
        );
    }

    let unresolved = "(ns reader.original)\n#_#test/change ::wire/payload\n(def saved :ok)";
    let report = session.check(path, unresolved);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX9000");
    assert_eq!(
        &unresolved[error.span.start.offset..error.span.end.offset],
        "::wire/payload"
    );

    let inactive = r#"(ns reader.original (:require [reader.prior :as-alias wire]))
      #?(:clj (def present :ready) :cljs #test/change :payload)
      (defn saved {:t/sig '[:-> #{:reader.prior/item}]} [] ::wire/item)"#;
    let report = session.check(path, inactive);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn a_literal_in_ns_changes_reader_data_after_the_complete_form() {
    let source = r#"(ns reader.original)
#?(:clj (in-ns 'reader.entered) :cljs ::inactive)
(def local #::{:label ::label})
(clojure.core/defn read-label {:t/sig '[:-> #{:reader.entered/label}]} [] ::label)
"#;
    let path = "reader/in_ns.cljc";
    let mut session = Session::default();
    let report = session.check(path, source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
    assert!(finding.message.contains("in-ns"));

    let early = source.replace("::inactive", "#{::inactive :reader.original/inactive}");
    let report = session.check(path, &early);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9000");
    assert_eq!(
        &early[finding.span.start.offset..finding.span.end.offset],
        ":reader.original/inactive"
    );

    let aliased = source.replace("#::{:label ::label}", "#::existing{:label ::label}");
    let report = session.check(path, &aliased);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
    assert!(finding.message.contains("unchecked after in-ns"));
    assert_eq!(
        &aliased[finding.span.start.offset..finding.span.end.offset],
        "existing"
    );

    let later = format!("{source}(def after ::unchecked)");
    let report = session.check(path, &later);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(finding.message.contains("namespace is unchecked"));
    assert_eq!(finding.code, "PNX9001");
    assert!(!report.complete, "{report:#?}");
    assert_eq!(
        &later[finding.span.start.offset..finding.span.end.offset],
        "::unchecked"
    );
}

#[test]
fn resolved_duplicate_literals_remain_errors_in_discarded_and_inactive_forms() {
    let ns = "(ns catalog.metadata (:require [catalog.items :as-alias items]))";
    let path = "catalog/metadata.cljc";
    for (body, repeated) in [
        (
            "[#_#{::local :catalog.metadata/local} :ok]",
            ":catalog.metadata/local",
        ),
        (
            "#?(:clj :ok :cljs #{::local :catalog.metadata/local})",
            ":catalog.metadata/local",
        ),
        (
            "#?(:cljs #::{:local 1 :catalog.metadata/local 2} :clj :ok)",
            ":catalog.metadata/local",
        ),
        (
            "[#_ [#_ {::items/key 1 :catalog.items/key 2} :discarded] :ok]",
            ":catalog.items/key",
        ),
        (
            "[#?@(:clj ^{::local 1 :catalog.metadata/local 2} [1])]",
            ":catalog.metadata/local",
        ),
    ] {
        let source = format!("{ns}\n{body}");
        let report = Session::default().check(path, &source);
        let [error] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(error.code, "PNX9000");
        assert!(!report.complete);
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            repeated
        );
    }
}

#[test]
fn reader_errors_in_inactive_provider_forms_invalidate_cached_calls() {
    let consumer = r#"(ns consumer (:require [provider :as p]))
(defn label {:t/sig '[:-> string?]} [] (p/total))"#;
    let provider = r#"(ns provider)
#?(:clj (def selected 0) :cljs #{::entry :other/entry})
(defn total {:t/sig '[:-> number?]} [] 7)"#;
    let broken = provider.replace(":other/entry", ":provider/entry");
    let check = |session: &mut Session, provider: &str| {
        session.check_project(
            &[
                Source {
                    path: "consumer.clj",
                    source: consumer,
                },
                Source {
                    path: "provider.cljc",
                    source: provider,
                },
            ],
            None,
        )
    };
    let mut session = Session::default();
    let before = check(&mut session, provider);
    assert!(before.complete, "{before:#?}");
    assert_eq!(before.files[0].diagnostics[0].code, "PNX2322");
    let after = check(&mut session, &broken);
    assert!(!after.complete, "{after:#?}");
    assert_eq!(after.files[0].diagnostics[0].code, "PNX9003");
    let reader_error = &after.files[1].diagnostics[0];
    assert_eq!(reader_error.code, "PNX9000");
    assert_eq!(
        &broken[reader_error.span.start.offset..reader_error.span.end.offset],
        ":provider/entry"
    );

    let cold = check(&mut Session::default(), &broken);
    let restored = check(&mut session, provider);
    for (warm, reference) in [
        (&after.files, &cold.files),
        (&restored.files, &before.files),
    ] {
        assert_eq!(warm.len(), reference.len());
        for (warm, reference) in warm.iter().zip(reference) {
            assert_eq!(warm.path, reference.path);
            assert_eq!(warm.complete, reference.complete);
            assert_eq!(
                serde_json::to_value(&warm.diagnostics).unwrap(),
                serde_json::to_value(&reference.diagnostics).unwrap()
            );
        }
    }
}

#[test]
fn map_alias_errors_remain_visible_when_the_value_is_discarded_or_inactive() {
    for source in [
        "(ns demo)\n#_ #::payload{:sku nil}",
        "(ns demo)\n#?(:clj nil :cljs #::payload{:sku nil})",
        "#::payload{:sku nil}\n(ns demo (:require [catalog.payload :as-alias payload]))",
    ] {
        let report = Session::default().check("namespace.cljc", source);
        let [error] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(error.code, "PNX9000");
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            "payload"
        );
    }
    let externs = Source {
        path: "api.edn",
        source: "#::{name [:-> string?]}",
    };
    let report = Session::default().check_project_with_externs(&[], &[externs]);
    assert!(!report.complete, "{report:#?}");
    assert!(
        report.files[0].diagnostics[0]
            .message
            .contains("Auto-resolved maps")
    );
}

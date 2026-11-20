use parenox_analysis::{Report, Session, Source};

fn specs() -> &'static str {
    include_str!("../../../tests/fixtures/spec-markup-content.clj")
        .split_once(";; JVM-only")
        .unwrap()
        .0
}

fn check(body: &str) -> Report {
    Session::default().check("markup_content.clj", &format!("{}{body}", specs()))
}

#[test]
fn named_string_set_checks_exact_input_values() {
    let mut uses = r#"
(defn markdown {:t/sig '[:-> #{"markdown"}]} [] (s/conform :markup/kind "markdown"))
(defn plaintext {:t/sig '[:-> #{"plaintext"}]} [] (s/conform ::kind "plaintext"))
(defn valid-markdown {:t/sig '[:-> #{true}]} [] (s/valid? ::kind "markdown"))
"#
    .to_owned();
    for (index, input) in [
        "\"\"",
        "\"Markdown\"",
        "\"markdown \"",
        ":markdown",
        "'markdown",
        "nil",
        "false",
        "true",
        "::s/invalid",
        "0",
        "[]",
        "{}",
    ]
    .into_iter()
    .enumerate()
    {
        uses.push_str(&format!(
            r#"
(defn invalid-{index} {{:t/sig '[:-> #{{::s/invalid}}]}} [] (s/conform :markup/kind {input}))
(defn invalid-{index}? {{:t/sig '[:-> #{{false}}]}} [] (s/valid? ::kind {input}))
"#
        ));
    }
    let report = check(&uses);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn broad_string_calls_keep_all_possible_results_and_narrow_true_branches() {
    let uses = r#"
(defn broad {:t/sig '[string? :-> #{"plaintext" "markdown" ::s/invalid}]} [x]
  (s/conform :markup/kind x))
(defn untyped {:t/sig '[any? :-> #{"plaintext" "markdown" ::s/invalid}]} [x]
  (s/conform :markup/kind x))
(defn disjoint {:t/sig '[keyword? :-> #{::s/invalid}]} [x]
  (s/conform :markup/kind x))
(defn broad-guarded {:t/sig '[string? :-> #{"plaintext" "markdown"}]} [x]
  (if (s/valid? ::kind x) x "plaintext"))
(defn finite-guarded {:t/sig '[#{"plaintext" "other"} :-> #{"plaintext"}]} [x]
  (if (s/valid? ::kind x) (s/conform :markup/kind x) "plaintext"))
"#;
    let report = check(uses);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{uses}
(defn always-invalid {{:t/sig '[string? :-> #{{false}}]}} [x] (s/valid? ::kind x))
(defn always-valid {{:t/sig '[string? :-> #{{true}}]}} [x] (s/valid? ::kind x))
(defn only-success {{:t/sig '[string? :-> #{{"plaintext" "markdown"}}]}} [x] (s/conform ::kind x))
(defn no-markdown {{:t/sig '[string? :-> #{{"plaintext" ::s/invalid}}]}} [x] (s/conform ::kind x))
(defn no-plaintext {{:t/sig '[string? :-> #{{"markdown" ::s/invalid}}]}} [x] (s/conform ::kind x))
"#
    ));
    let expected = [
        "always-invalid",
        "always-valid",
        "only-success",
        "no-markdown",
        "no-plaintext",
    ];
    assert_eq!(report.diagnostics.len(), expected.len(), "{report:#?}");
    for (finding, name) in report.diagnostics.iter().zip(expected) {
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().name,
            format!("demo.markup-content/{name}"),
            "{finding:#?}"
        );
    }
}

#[test]
fn markup_contents_preserve_map_entries_and_check_registered_fields() {
    let uses = r#"
(def left {:kind "markdown" :value "contents" :other nil})
(def right {:kind "plaintext" :value "plain" :markup/kind "markdown"})
(defn kept-key {:t/sig '[:-> #{"markdown"}]} []
  (:kind (key (s/conform ::contents (clojure.lang.MapEntry/create left right)))))
(defn kept-extra {:t/sig '[:-> #{"markdown"}]} []
  (:markup/kind (val (s/conform ::contents (clojure.lang.MapEntry/create left right)))))
(defn missing-kind {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::markup-content {:value "x" :extra/unregistered false}))
(defn missing-value {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::markup-content {:kind "markdown"}))
(defn invalid-extra {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::markup-content {:kind "markdown" :value "x" :markup/kind "other"}))
(defn invalid-row {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::contents (clojure.lang.MapEntry/create left {:kind "markdown" :value ::s/invalid})))
"#;
    let report = check(uses);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{uses}
(defn undeclared {{:t/sig '[:-> any?]}} []
  (s/conform ::markup-content {{:kind "markdown" :value "x" :extra/unregistered false}}))
"#
    ));
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9001", "{finding:#?}");
    assert!(finding.message.contains("undeclared qualified key"));
}

#[test]
fn mixed_sets_keep_their_custom_conformance_gap() {
    let body = r#"
(s/def ::mixed #{"markdown" nil false ::s/invalid})
(defn unchecked {:t/sig '[:-> any?]} [] (s/conform ::mixed "markdown"))
"#;
    let source = format!("{}{body}", specs());
    let report = Session::default().check("markup_content.clj", &source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9001", "{finding:#?}");
    assert!(finding.message.contains("supported conversion"));
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "::mixed"
    );
}

#[test]
fn markup_contracts_import_without_source_and_reject_a_broad_string() {
    let provider = format!(
        r#"{}
(defn kind {{:t/sig '[:markup/kind :-> string?]}} [x] x)
(defn render {{:t/sig '[::markup-content :-> string?]}} [x] (:value x))
"#,
        specs()
    );
    let provider = Source {
        path: "provider.clj",
        source: &provider,
    };
    let mut session = Session::default();
    let exported = session.export_externs(&[provider], &[]);
    assert!(
        exported.report.ok && exported.report.complete,
        "{exported:#?}"
    );
    let edn = exported.edn.unwrap();
    let externs = Source {
        path: "provider.edn",
        source: &edn,
    };
    let validation = session.validate_externs(&[provider], &[externs]);
    assert!(validation.ok && validation.complete, "{validation:#?}");
    let check = |body: &str| {
        let consumer = format!(
            "(ns consumer (:require [clojure.spec.alpha :as s] [demo.markup-content :as wire]))\n{body}"
        );
        Session::default().check_project_with_externs(
            &[Source {
                path: "consumer.clj",
                source: &consumer,
            }],
            &[externs],
        )
    };
    let accepted = check(
        r#"(defn send {:t/sig '[:-> string?]} []
             (wire/kind "plaintext")
             (wire/render {:kind "markdown" :value "escaped\ntext"}))"#,
    );
    assert!(accepted.ok && accepted.complete, "{accepted:#?}");

    for body in [
        r#"(defn sends {:t/sig '[:-> string?]} [] (wire/kind "Markdown"))"#,
        r#"(defn sends {:t/sig '[string? :-> string?]} [x] (wire/kind x))"#,
        r#"(defn sends {:t/sig '[:-> string?]} [] (wire/render {:kind "other" :value "x"}))"#,
        r#"(defn sends {:t/sig '[:-> string?]} [] (wire/render {:kind "markdown" :value ::s/invalid}))"#,
    ] {
        let report = check(body);
        let [finding] = report.files[0].diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().path.as_deref(),
            Some("provider.edn")
        );
    }
}

#[test]
fn moving_a_markup_registration_rechecks_only_unchanged_dependents() {
    let registration = "(s/def :markup/kind #{\"plaintext\" \"markdown\"})";
    let consumer = r#"
(ns consumer (:require [clojure.spec.alpha :as s] [demo.markup-content :as wire]))
(defn direct {:t/sig '[:-> #{"markdown"}]} [] (s/conform :markup/kind "markdown"))
(defn nested {:t/sig '[:-> #{"markdown"}]} []
  (:kind (s/conform ::wire/markup-content {:kind "markdown" :value "x"})))
(defn independent {:t/sig '[:-> string?]} [] "stable")
"#;
    let moved_provider = specs().replace(registration, "");
    let new_owner =
        format!("(ns demo.unloaded (:require [clojure.spec.alpha :as s]))\n{registration}");
    let check = |session: &mut Session, source: &str, moved: bool| {
        let mut sources = vec![
            Source {
                path: "consumer.clj",
                source,
            },
            Source {
                path: "demo/markup_content.clj",
                source: if moved { &moved_provider } else { specs() },
            },
        ];
        if moved {
            sources.push(Source {
                path: "demo/unloaded.clj",
                source: &new_owner,
            });
        }
        session.check_project(&sources, None).files.remove(0)
    };
    let assert_no_load = |report: &Report, source: &str| {
        let blocked: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|finding| {
                finding.code == "PNX9001" && finding.message.contains("registrations are loaded")
            })
            .map(|finding| &source[finding.span.start.offset..finding.span.end.offset])
            .collect();
        assert_eq!(
            blocked,
            [":markup/kind", "::wire/markup-content"],
            "{report:#?}"
        );
    };

    let mut session = Session::default();
    let aliased = consumer.replace(":as wire", ":as-alias wire");
    assert_no_load(&check(&mut session, &aliased, false), &aliased);
    let loaded = check(&mut session, consumer, false);
    assert!(loaded.ok && loaded.complete, "{loaded:#?}");
    let warm = check(&mut session, consumer, false);
    assert_eq!(warm.stats.reused_functions, 3, "{warm:#?}");

    let moved = check(&mut session, consumer, true);
    assert_no_load(&moved, consumer);
    assert_eq!(moved.stats.checked_functions, 2, "{moved:#?}");
    assert_eq!(moved.stats.reused_functions, 1, "{moved:#?}");

    let restored = check(&mut session, consumer, false);
    assert!(restored.ok && restored.complete, "{restored:#?}");
    assert_eq!(restored.stats.checked_functions, 2, "{restored:#?}");
    assert_eq!(restored.stats.reused_functions, 1, "{restored:#?}");
}

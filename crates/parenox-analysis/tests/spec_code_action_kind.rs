use parenox_analysis::{Report, Session, Source};

fn specs() -> &'static str {
    include_str!("../../../tests/fixtures/spec-code-action-kind.clj")
        .split_once(";; JVM-only")
        .unwrap()
        .0
}

fn check(body: &str) -> Report {
    Session::default().check("code_action_kind.clj", &format!("{}{body}", specs()))
}

#[test]
fn known_keywords_map_to_strings_and_arbitrary_strings_pass_in_both_orders() {
    let mut body = r#"
(s/def ::string-first
  (s/and (s/or :string string?
               :keyword (s/and keyword?
                               demo.code-action-kind/code-action-kind
                               (s/conformer demo.code-action-kind/code-action-kind)))
         (s/conformer clojure.core/second)))
(defn mapped {:t/sig '[:-> #{"quickfix"}]} [] (s/conform :code-action/kind :quick-fix))
(defn alias {:t/sig '[:-> #{"refactor.inline"}]} [] (s/conform ::kind :refactor-inline))
(defn reverse {:t/sig '[:-> #{"source.organizeImports"}]} []
  (s/conform ::string-first :source-organize-imports))
(defn raw {:t/sig '[:-> #{"workspace.myAction"}]} []
  (s/conform :code-action/kind "workspace.myAction"))
(defn reverse-raw {:t/sig '[:-> #{":quick-fix"}]} []
  (s/conform ::string-first ":quick-fix"))
(defn mapped-valid {:t/sig '[:-> #{true}]} [] (s/valid? ::string-first :quick-fix))
(defn empty-valid {:t/sig '[:-> #{true}]} [] (s/valid? :code-action/kind ""))
(defn empty-mapped {:t/sig '[:-> #{""}]} [] (s/conform ::edge-kind :empty))
(defn empty-mapped-valid {:t/sig '[:-> #{true}]} [] (s/valid? ::edge-kind :empty))
"#
    .to_owned();
    for (index, (spec, input)) in [
        (":code-action/kind", ":unlisted"),
        (":code-action/kind", ":foreign/quick-fix"),
        (":code-action/kind", "nil"),
        (":code-action/kind", "false"),
        (":code-action/kind", "1"),
        (":code-action/kind", "'quick-fix"),
        (":code-action/kind", "::s/invalid"),
        ("::string-first", ":unlisted"),
        ("::edge-kind", ":false"),
        ("::edge-kind", ":nil"),
        ("::edge-kind", ":marker"),
        ("::edge-kind", "::s/invalid"),
    ]
    .into_iter()
    .enumerate()
    {
        body.push_str(&format!(
            r#"
(defn invalid-{index} {{:t/sig '[:-> #{{::s/invalid}}]}} []
  (s/conform {spec} {input}))
(defn invalid-{index}? {{:t/sig '[:-> #{{false}}]}} []
  (s/valid? {spec} {input}))
"#
        ));
    }
    let report = check(&body);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn broad_string_succeeds_and_broad_keyword_has_mapped_and_invalid_outcomes() {
    let body = r#"
(defn raw {:t/sig '[string? :-> string?]} [kind] (s/conform :code-action/kind kind))
(defn raw-valid {:t/sig '[string? :-> #{true}]} [kind] (s/valid? :code-action/kind kind))
(defn all-keyword-outputs
  {:t/sig '[keyword? :-> #{"quickfix" "refactor" "refactor.extract" "refactor.inline"
                           "refactor.rewrite" "source" "source.organizeImports" ::s/invalid}]}
  [kind] (s/conform :code-action/kind kind))
(defn all-values {:t/sig '[any? :-> (s/or :kind string? :invalid #{::s/invalid})]} [kind]
  (s/conform :code-action/kind kind))
(defn edge {:t/sig '[keyword? :-> #{"" ::s/invalid}]} [kind] (s/conform ::edge-kind kind))
(defn empty-only {:t/sig '[#{:empty} :-> #{:empty}]} [kind] kind)
(defn guarded {:t/sig '[#{:empty :false :nil :marker} :-> #{:empty}]} [kind]
  (if (s/valid? ::edge-kind kind) (empty-only kind) :empty))
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{body}
(defn always-valid {{:t/sig '[keyword? :-> #{{true}}]}} [kind]
  (s/valid? :code-action/kind kind))
(defn always-invalid {{:t/sig '[keyword? :-> #{{false}}]}} [kind]
  (s/valid? :code-action/kind kind))
(defn no-success {{:t/sig '[keyword? :-> #{{::s/invalid}}]}} [kind]
  (s/conform :code-action/kind kind))
(defn no-marker {{:t/sig '[keyword? :-> string?]}} [kind]
  (s/conform :code-action/kind kind))
"#
    ));
    let names = ["always-valid", "always-invalid", "no-success", "no-marker"];
    assert_eq!(report.diagnostics.len(), names.len(), "{report:#?}");
    for (finding, name) in report.diagnostics.iter().zip(names) {
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().name,
            format!("demo.code-action-kind/{name}")
        );
    }
}

#[test]
fn nested_map_conformance_preserves_string_entries_and_rebuilds_mapped_entries() {
    let body = r#"
(def raw {:title "Existing kind" :kind "workspace.myAction"})
(def omitted {:title "No kind"})
(def mapped {:title "Organize imports" :kind :source-organize-imports})
(defn preserved-key {:t/sig '[:-> #{"workspace.myAction"}]} []
  (:kind (key (s/conform ::code-actions (clojure.lang.MapEntry/create raw omitted)))))
(defn preserved-value {:t/sig '[:-> #{"No kind"}]} []
  (:title (val (s/conform ::code-actions (clojure.lang.MapEntry/create raw omitted)))))
(defn changed-value {:t/sig '[:-> #{"source.organizeImports"}]} []
  (:kind (first (rest (s/conform ::code-actions (clojure.lang.MapEntry/create raw mapped))))))
(defn nested {:t/sig '[:-> #{"source.organizeImports"}]} []
  (:kind (:code-action (s/conform ::action-batch {:code-action mapped}))))
(defn missing-title {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::code-action {:kind :source-organize-imports}))
(defn unknown-kind {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::code-action {:title "Unknown kind" :kind :unlisted}))
(defn nil-kind {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::code-action {:title "Nil kind" :kind nil}))
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{body}
(defn rebuilt {{:t/sig '[:-> any?]}} []
  (key (s/conform ::code-actions (clojure.lang.MapEntry/create raw mapped))))
"#
    ));
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert!(finding.message.contains("key"), "{finding:#?}");
}

#[test]
fn only_same_file_literal_tables_and_resolved_core_predicates_are_proven() {
    let source = r#"
(ns demo.code-action-kind-opaque
  (:refer-clojure :exclude [keyword? second])
  (:require [clojure.spec.alpha :as s]))
(defn keyword? {:t/sig '[any? :-> boolean?]} [_] true)
(defn second {:t/sig '[any? :-> string?]} [_] "shadow")
(def kinds {:known "known"})
(def computed (assoc {} :known "known"))
(s/def ::computed
  (s/and (s/or :kw (s/and clojure.core/keyword? computed (s/conformer computed))
               :str string?)
         (s/conformer clojure.core/second)))
(s/def ::shadowed-guard
  (s/and (s/or :kw (s/and keyword? kinds (s/conformer kinds))
               :str string?)
         (s/conformer clojure.core/second)))
(s/def ::shadowed-projection
  (s/and (s/or :kw (s/and clojure.core/keyword? kinds (s/conformer kinds))
               :str string?)
         (s/conformer second)))
(s/def ::canonical
  (s/and (s/or :kw (s/and clojure.core/keyword? kinds (s/conformer kinds))
               :str string?)
         (s/conformer clojure.core/second)))
(defn a {:t/sig '[:-> any?]} [] (s/conform ::computed :known))
(defn b {:t/sig '[:-> any?]} [] (s/conform ::shadowed-guard :known))
(defn c {:t/sig '[:-> any?]} [] (s/conform ::shadowed-projection :known))
(defn d {:t/sig '[:-> #{"known"}]} [] (s/conform ::canonical :known))
"#;
    let report = Session::default().check("code_action_kind_opaque.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .all(|finding| finding.code == "PNX9001"),
        "{report:#?}"
    );
    let unsupported: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.message.contains("supported conversion"))
        .map(|finding| &source[finding.span.start.offset..finding.span.end.offset])
        .collect();
    assert_eq!(
        unsupported,
        ["::computed", "::shadowed-guard", "::shadowed-projection"],
        "{report:#?}"
    );
}

#[test]
fn code_action_kind_input_contract_round_trips_without_provider_source() {
    let provider = format!(
        r#"{}
(defn accept-kind {{:t/sig '[::kind :-> keyword?]}} [_] :accepted)
(defn accept-action {{:t/sig '[::code-action :-> keyword?]}} [_] :accepted)
"#,
        specs()
    );
    let exported = Session::default().export_externs(
        &[Source {
            path: "provider.clj",
            source: &provider,
        }],
        &[],
    );
    assert!(
        exported.report.ok && exported.report.complete,
        "{exported:#?}"
    );
    let edn = exported.edn.unwrap();
    let check = |body: &str| {
        let source = format!(
            "(ns consumer (:require [demo.code-action-kind :as wire] [clojure.spec.alpha :as s]))\n{body}"
        );
        Session::default().check_project_with_externs(
            &[Source {
                path: "consumer.clj",
                source: &source,
            }],
            &[Source {
                path: "provider.edn",
                source: &edn,
            }],
        )
    };
    let accepted = check(
        r#"
(defn sends {:t/sig '[string? :-> keyword?]} [kind]
  (wire/accept-kind :quick-fix)
  (wire/accept-kind kind)
  (wire/accept-action {:title "Existing" :kind kind})
  (wire/accept-action {:title "Extract" :kind :refactor-extract})
  (wire/accept-action {:title "No kind"}))
"#,
    );
    assert!(accepted.ok && accepted.complete, "{accepted:#?}");
    for call in [
        "(wire/accept-kind :unlisted)",
        "(wire/accept-kind ::s/invalid)",
        "(wire/accept-kind nil)",
        r#"(wire/accept-action {:title "Unknown" :kind :unlisted})"#,
    ] {
        let rejected = check(&format!(
            r#"(defn sends {{:t/sig '[:-> keyword?]}} [] {call})"#
        ));
        let [finding] = rejected.files[0].diagnostics.as_slice() else {
            panic!("{rejected:#?}");
        };
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().path.as_deref(),
            Some("provider.edn")
        );
    }
}

#[test]
fn a_table_edit_rechecks_an_unchanged_consumer_and_owner_loading_is_required() {
    let source = r#"
(ns consumer (:require [clojure.spec.alpha :as s] [demo.code-action-kind :as kinds]))
(defn quick-fix {:t/sig '[:-> #{"quickfix"}]} [] (s/conform ::kinds/kind :quick-fix))
(defn independent {:t/sig '[:-> string?]} [] "independent")
"#;
    let check = |session: &mut Session, consumer: &str, provider: &str| {
        session
            .check_project(
                &[
                    Source {
                        path: "consumer.clj",
                        source: consumer,
                    },
                    Source {
                        path: "demo/code_action_kind.clj",
                        source: provider,
                    },
                ],
                None,
            )
            .files
            .into_iter()
            .find(|file| file.path == "consumer.clj")
            .unwrap()
    };
    let mut session = Session::default();
    let alias_only = source.replace(":as kinds", ":as-alias kinds");
    let unavailable = check(&mut session, &alias_only, specs());
    let [gap] = unavailable.diagnostics.as_slice() else {
        panic!("{unavailable:#?}");
    };
    assert_eq!(gap.code, "PNX9001", "{gap:#?}");
    assert!(gap.message.contains("registrations are loaded"), "{gap:#?}");
    for _ in 0..2 {
        let report = check(&mut session, source, specs());
        assert!(report.ok && report.complete, "{report:#?}");
    }

    let updated = specs().replace(r#":quick-fix "quickfix""#, r#":quick-fix "fix.updated""#);
    let changed = check(&mut session, source, &updated);
    let [finding] = changed.diagnostics.as_slice() else {
        panic!("{changed:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    let fresh = check(&mut Session::default(), source, &updated);
    assert_eq!(fresh.diagnostics.len(), 1, "{fresh:#?}");
    assert_eq!(fresh.diagnostics[0].message, finding.message);
    let restored = check(&mut session, source, specs());
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

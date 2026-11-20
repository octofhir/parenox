use parenox_analysis::{Report, Session, Source};

fn shared_specs() -> &'static str {
    include_str!("../../../tests/fixtures/spec-file-kind.clj")
        .split_once(";; JVM-only")
        .unwrap()
        .0
}

fn specs() -> String {
    // The lsp4clj source leaves these option registrations open. Give the
    // standalone export fixture explicit contracts for their values.
    format!(
        "{}\n(s/def ::overwrite any?)\n(s/def ::ignore-if-exists any?)\n",
        shared_specs()
    )
}

fn check(body: &str) -> Report {
    Session::default().check("file_kind.clj", &format!("{}{body}", specs()))
}

#[test]
fn exact_file_kind_checks_the_same_strings_as_the_jvm() {
    let mut body = r#"
(defn create-kind {:t/sig '[:-> #{"create"}]} []
  (s/conform :create-file/kind "create"))
(defn rename-kind {:t/sig '[:-> #{"rename"}]} []
  (s/conform ::rename-kind "rename"))
(defn bare-rename-valid {:t/sig '[:-> #{true}]} []
  (s/valid? :rename-file/kind "rename"))
(defn reversed-percent-one {:t/sig '[:-> #{"create"}]} []
  (s/conform ::reversed "create"))
(defn supplementary-scalar {:t/sig '[:-> #{"🧪"}]} []
  (s/conform ::emoji "🧪"))
(defn escaped-pair {:t/sig '[:-> #{true}]} []
  (s/valid? ::emoji "\ud83e\uddea"))
(defn escaped-accent {:t/sig '[:-> #{"é"}]} []
  (s/conform ::accent "\u00e9"))
(defn decomposed-accent {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::accent "e\u0301"))
"#
    .to_owned();
    for (index, input) in [
        r#""rename""#,
        r#""Create""#,
        r#""create ""#,
        r#""""#,
        ":create",
        "'create",
        r#"\c"#,
        "nil",
        "false",
        "1",
        "[]",
        "::s/invalid",
    ]
    .into_iter()
    .enumerate()
    {
        body.push_str(&format!(
            r#"
(defn invalid-{index} {{:t/sig '[:-> #{{::s/invalid}}]}} []
  (s/conform :create-file/kind {input}))
(defn invalid-{index}? {{:t/sig '[:-> #{{false}}]}} []
  (s/valid? :create-file/kind {input}))
"#
        ));
    }
    let report = check(&body);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn broad_strings_can_succeed_or_fail_and_a_true_guard_narrows_to_the_literal() {
    let body = r#"
(defn create-only {:t/sig '[#{"create"} :-> #{"create"}]} [kind] kind)
(defn broad-conform {:t/sig '[string? :-> #{"create" ::s/invalid}]} [kind]
  (s/conform :create-file/kind kind))
(defn guarded {:t/sig '[string? :-> #{"create"}]} [kind]
  (if (s/valid? :create-file/kind kind)
    (create-only kind)
    "create"))
(defn guarded-finite {:t/sig '[#{"create" "rename"} :-> #{"create"}]} [kind]
  (if (s/valid? :create-file/kind kind) (s/conform :create-file/kind kind) "create"))
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{body}
(defn always-valid {{:t/sig '[string? :-> #{{true}}]}} [kind]
  (s/valid? :create-file/kind kind))
(defn always-invalid {{:t/sig '[string? :-> #{{false}}]}} [kind]
  (s/valid? :create-file/kind kind))
(defn only-create {{:t/sig '[string? :-> #{{"create"}}]}} [kind]
  (s/conform :create-file/kind kind))
(defn only-marker {{:t/sig '[string? :-> #{{::s/invalid}}]}} [kind]
  (s/conform :create-file/kind kind))
"#
    ));
    let expected = [
        "always-valid",
        "always-invalid",
        "only-create",
        "only-marker",
    ];
    assert_eq!(report.diagnostics.len(), expected.len(), "{report:#?}");
    for (finding, name) in report.diagnostics.iter().zip(expected) {
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().name,
            format!("demo.file-kind/{name}")
        );
    }
}

#[test]
fn identity_conformance_keeps_nested_map_entries_and_checks_the_operation_kind() {
    let report = check(
        r#"
(def create {:kind "create" :uri "file:///created"})
(def rename {:kind "rename" :old-uri "file:///old" :new-uri "file:///new"})
(defn created-kind {:t/sig '[:-> #{"create"}]} [] (:kind (s/conform ::create-file create)))
(defn renamed-key {:t/sig '[:-> #{"rename"}]} []
  (:kind (key (s/conform ::renames (clojure.lang.MapEntry/create rename rename)))))
(defn renamed-value {:t/sig '[:-> #{"rename"}]} []
  (:kind (val (s/conform ::renames (clojure.lang.MapEntry/create rename rename)))))
(defn missing-kind {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::create-file {:uri "file:///created"}))
(defn wrong-kind {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::rename-file {:kind "create" :old-uri "file:///old" :new-uri "file:///new"}))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn only_one_literal_comparison_using_core_equality_is_proven() {
    let source = r#"
(ns demo.file-kind-opaque
  (:refer-clojure :exclude [=])
  (:require [clojure.spec.alpha :as s]))
(defn = {:t/sig '[any? any? :-> boolean?]} [_ _] true)
(s/def ::shadowed (s/and string? #(= % "create")))
(s/def ::arbitrary (s/and string? #(or (clojure.core/= % "create") (clojure.core/= % "rename"))))
(s/def ::extra-operand (s/and string? #(clojure.core/= % "create" "create")))
(s/def ::unpaired-surrogate (s/and string? #(clojure.core/= % "\ud800")))
(s/def ::canonical (s/and string? #(clojure.core/= % "create")))
(defn a {:t/sig '[:-> any?]} [] (s/conform ::shadowed "rename"))
(defn b {:t/sig '[:-> any?]} [] (s/conform ::arbitrary "rename"))
(defn c {:t/sig '[:-> any?]} [] (s/conform ::extra-operand "create"))
(defn d {:t/sig '[:-> any?]} [] (s/conform ::unpaired-surrogate "\ud800"))
(defn e {:t/sig '[:-> #{::s/invalid}]} [] (s/conform ::canonical "rename"))
"#;
    let report = Session::default().check("file_kind_opaque.clj", source);
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
        [
            "::shadowed",
            "::arbitrary",
            "::extra-operand",
            "::unpaired-surrogate"
        ],
        "{report:#?}"
    );
}

#[test]
fn a_signature_using_a_singleton_kind_map_imports_without_provider_source() {
    let provider = format!(
        r#"{}
(defn rename {{:t/sig '[::rename-file :-> string?]}} [entry] (:new-uri entry))
"#,
        specs()
    );
    let provider = Source {
        path: "provider.clj",
        source: &provider,
    };
    let exported = Session::default().export_externs(&[provider], &[]);
    assert!(
        exported.report.ok && exported.report.complete,
        "{exported:#?}"
    );
    let edn = exported.edn.unwrap();
    let externs = Source {
        path: "provider.edn",
        source: &edn,
    };
    let consumer = |body: &str| {
        let source = format!("(ns consumer (:require [demo.file-kind :as wire]))\n{body}");
        Session::default().check_project_with_externs(
            &[Source {
                path: "consumer.clj",
                source: &source,
            }],
            &[externs],
        )
    };
    let accepted = consumer(
        r#"(defn send {:t/sig '[:-> string?]} []
             (wire/rename {:kind "rename" :old-uri "file:///old" :new-uri "file:///new"}))"#,
    );
    assert!(accepted.ok && accepted.complete, "{accepted:#?}");
    for body in [
        r#"(defn send {:t/sig '[:-> string?]} []
             (wire/rename {:kind "create" :old-uri "file:///old" :new-uri "file:///new"}))"#,
        r#"(defn send {:t/sig '[string? :-> string?]} [kind]
             (wire/rename {:kind kind :old-uri "file:///old" :new-uri "file:///new"}))"#,
    ] {
        let rejected = consumer(body);
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
fn changing_the_literal_rechecks_an_unchanged_conform_caller() {
    let consumer = Source {
        path: "consumer.clj",
        source: r#"
(ns consumer (:require [clojure.spec.alpha :as s] [demo.kind :as kind]))
(defn create-kind {:t/sig '[:-> #{"create"}]} [] (s/conform ::kind/resource "create"))
"#,
    };
    let check = |session: &mut Session, kind: &str| {
        let provider = format!(
            r#"(ns demo.kind (:require [clojure.spec.alpha :as s]))
(s/def ::resource (s/and string? #(= % "{kind}")))"#
        );
        session
            .check_project(
                &[
                    consumer,
                    Source {
                        path: "demo/kind.clj",
                        source: &provider,
                    },
                ],
                None,
            )
            .files
            .into_iter()
            .find(|file| file.path == consumer.path)
            .unwrap()
    };
    let mut session = Session::default();
    for _ in 0..2 {
        let result = check(&mut session, "create");
        assert!(result.ok && result.complete, "{result:#?}");
    }
    let changed = check(&mut session, "rename");
    let [finding] = changed.diagnostics.as_slice() else {
        panic!("{changed:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    let restored = check(&mut session, "create");
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

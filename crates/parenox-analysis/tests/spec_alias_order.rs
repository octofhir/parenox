use parenox_analysis::{Session, Source};

const HEADER: &str = "(ns demo.alias-order (:require [clojure.spec.alpha :as s]))";
const ALIAS: &str = "(s/def ::alias ::target)";
const TARGET: &str = r#"(s/def ::target (s/and string? #(= % "ready")))"#;

#[test]
fn a_forward_alias_cannot_prove_conformance_or_export_a_signature() {
    let source = format!(
        r#"{HEADER}
{ALIAS}
{TARGET}
(defn conforms {{:t/sig '[:-> #{{"ready"}}]}} [] (s/conform ::alias "ready"))
(defn accepts {{:t/sig '[::alias :-> #{{"ready"}}]}} [kind] kind)
"#
    );
    let provider = Source {
        path: "alias_order.clj",
        source: &source,
    };
    let mut session = Session::default();
    let report = session.check(provider.path, provider.source);
    assert!(!report.ok && !report.complete, "{report:#?}");
    let finding = report
        .diagnostics
        .iter()
        .find(|finding| {
            finding
                .message
                .contains("requires :demo.alias-order/target to be registered first")
        })
        .expect("registration warning");
    assert_eq!(finding.code, "PNX9001");
    assert_eq!(finding.span.start.line, 2);
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "::target"
    );
    assert!(report.diagnostics.iter().any(|finding| {
        finding
            .message
            .contains("requires a named spec with a supported conversion")
    }));
    assert!(session.export_externs(&[provider], &[]).edn.is_none());
}

#[test]
fn a_prior_map_registration_can_be_aliased_before_its_field_is_registered() {
    let source = format!(
        r#"{HEADER}
(s/def ::target (s/keys :req-un [::label]))
{ALIAS}
(s/def ::chain ::alias)
(s/def ::label string?)
(defn accepts {{:t/sig '[::chain :-> string?]}} [entry] (:label entry))
(defn conforms {{:t/sig '[:-> #{{"ready"}}]}} []
  (:label (s/conform ::chain {{:label "ready"}})))
(defn invalid {{:t/sig '[:-> #{{::s/invalid}}]}} []
  (s/conform ::chain {{:label nil}}))
"#
    );
    let provider = Source {
        path: "alias_order.clj",
        source: &source,
    };
    let exported = Session::default().export_externs(&[provider], &[]);
    assert!(
        exported.report.ok && exported.report.complete,
        "{exported:#?}"
    );
    assert!(exported.edn.unwrap().contains("demo.alias-order/accepts"));
}

#[test]
fn reordering_alias_registration_refreshes_an_unchanged_consumer() {
    let source = Source {
        path: "consumer.clj",
        source: r#"
(ns consumer (:require [clojure.spec.alpha :as s] [demo.alias-order :as wire]))
(defn reads-kind {:t/sig '[:-> #{"ready"}]} [] (s/conform ::wire/alias "ready"))
"#,
    };
    let ordered = format!("{HEADER}\n{TARGET}\n{ALIAS}");
    let forward = format!("{HEADER}\n{ALIAS}\n{TARGET}");
    let check = |session: &mut Session, provider| {
        session.check_project(
            &[
                source,
                Source {
                    path: "alias_order.clj",
                    source: provider,
                },
            ],
            None,
        )
    };
    let mut session = Session::default();
    let initial = check(&mut session, &ordered);
    assert!(initial.ok && initial.complete, "{initial:#?}");
    let changed = check(&mut session, &forward);
    assert!(!changed.complete, "{changed:#?}");
    let consumer = &changed.files[0];
    assert_eq!(consumer.stats.reused_parses, 1, "{changed:#?}");
    assert_eq!(consumer.stats.checked_functions, 1, "{changed:#?}");
    assert_eq!(consumer.diagnostics.len(), 1, "{changed:#?}");
    assert_eq!(consumer.diagnostics[0].code, "PNX9001");
    assert!(
        changed.files[1]
            .diagnostics
            .iter()
            .any(|finding| { finding.message.contains("to be registered first") })
    );
    let restored = check(&mut session, &ordered);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

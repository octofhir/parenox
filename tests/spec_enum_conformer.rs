use parenox::{ProjectReport, Session, Source};

fn wire() -> &'static str {
    include_str!("fixtures/spec-enum-conformer.clj")
        .split_once(";; JVM-only assertions:")
        .unwrap()
        .0
}

fn check(session: &mut Session, wire: &str, adapter: &str) -> ProjectReport {
    session.check_project(
        &[
            Source {
                path: "coercer.clj",
                source: wire,
            },
            Source {
                path: "adapter.clj",
                source: adapter,
            },
        ],
        None,
    )
}

#[test]
fn checks_typed_file_event_inputs_and_keeps_unknown_keys_unproven() {
    let adapter = r#"(ns adapter (:require [clojure.spec.alpha :as s]))
(s/def ::event (s/keys :req-un [:file-event/type]))
(s/fdef accept :args (s/cat :event ::event) :ret keyword?)
(defn accept [event] :accepted)
(defn send {:t/sig '[:-> keyword?]} [] (accept {:type 1}))
"#;
    let mut session = Session::default();
    let good = check(&mut session, wire(), adapter);
    assert!(good.ok && good.complete, "{good:#?}");
    for value in ["4", "1N", "1.0", "nil", "\"1\"", "2147483648"] {
        let adapter = adapter.replace("{:type 1}", &format!("{{:type {value}}}"));
        let report = check(&mut session, wire(), &adapter);
        let [finding] = report.files[1].diagnostics.as_slice() else {
            panic!("{value}: {report:#?}")
        };
        assert!(report.complete, "{value}: {report:#?}");
        assert_eq!(finding.code, "PNX2322", "{value}: {finding:#?}");
        assert!(finding.message.contains(":type"), "{finding:#?}");
    }
    let unknown = r#"(ns adapter)
(defn accept {:t/sig '[:file-event/type :-> keyword?]} [x] :accepted)
(defn send {:t/sig '[int? :-> keyword?]} [x] (accept x))
"#;
    let report = check(&mut session, wire(), unknown);
    let [finding] = report.files[1].diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert!(!report.complete);
    assert_eq!(finding.code, "PNX9004");
}

#[test]
fn conformation_maps_exact_keys_and_validation_refines_the_original_input() {
    let adapter = r#"(ns adapter (:require [clojure.spec.alpha :as s]))
(s/def ::alias :file-event/type)
(defn created {:t/sig '[:-> #{:created}]} [] (s/conform ::alias 1))
(defn invalid {:t/sig '[:-> #{:clojure.spec.alpha/invalid}]} [] (s/conform ::alias 1N))
(defn invalid-large {:t/sig '[:-> #{:clojure.spec.alpha/invalid}]} []
  (s/conform ::alias 2147483648))
(defn changed? {:t/sig '[:-> #{true}]} [] (s/valid? ::alias 2))
(defn no-event? {:t/sig '[:-> #{false}]} [] (s/valid? ::alias 4))
(defn guarded {:t/sig '[int? :-> #{:created :changed :deleted}]} [x]
  (if (s/valid? ::alias x) (s/conform ::alias x) :deleted))
(defn selected {:t/sig '[boolean? :-> #{:created}]} [flag]
  (let [x (if flag 1 4)]
    (if (s/valid? ::alias x) (s/conform ::alias x) :created)))
"#;
    let report = check(&mut Session::default(), wire(), adapter);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn enum_entries_must_produce_a_truthy_non_invalid_value() {
    let source = r#"(ns example (:require [clojure.spec.alpha :as s]))
(def states {1 false 2 nil 3 ::s/invalid 4 :ok})
(s/def ::state (s/and int? states (s/conformer states)))
(defn invalid {:t/sig '[:-> #{:clojure.spec.alpha/invalid}]} []
  (s/conform ::state 1)
  (s/conform ::state 2)
  (s/conform ::state 3))
(defn falsey {:t/sig '[:-> #{false}]} []
  (or (s/valid? ::state 1) (s/valid? ::state 2) (s/valid? ::state 3)))
(defn present {:t/sig '[:-> #{:ok}]} [] (s/conform ::state 4))
(defn lookup {:t/sig '[:-> #{:ok}]} [] (get states 4))
(defn invoke {:t/sig '[:-> #{:ok}]} [] (states 4))
"#;
    let report = Session::default().check("states.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let source = source.replace("[:-> #{:ok}]} [] (states 4)", "[:-> nil?]} [] (states 4N)");
    let report = Session::default().check("states.clj", &source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{report:#?}");
}

#[test]
fn tracks_changes_to_the_lookup_keys_and_outputs_in_project_sessions() {
    let adapter = r#"(ns adapter (:require [clojure.spec.alpha :as s]))
(defn accepts {:t/sig '[:file-event/type :-> keyword?]} [x] :accepted)
(defn send {:t/sig '[:-> keyword?]} [] (accepts 1))
(defn created {:t/sig '[:-> #{:created}]} [] (s/conform :file-event/type 1))
"#;
    let mut session = Session::default();
    let initial = check(&mut session, wire(), adapter);
    assert!(initial.ok && initial.complete, "{initial:#?}");
    let output_changed = wire().replace(":created", ":removed");
    let report = check(&mut session, &output_changed, adapter);
    let [finding] = report.files[1].diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some(":removed"));
    let key_changed = wire().replace("{1 ", "{9 ");
    let report = check(&mut session, &key_changed, adapter);
    assert_eq!(report.files[1].diagnostics.len(), 2, "{report:#?}");
    let restored = check(&mut session, wire(), adapter);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn dynamic_or_ambiguous_enum_patterns_stay_unchecked() {
    for source in [
        wire().replace(
            "(def file-change-type-enum",
            "(def ^:dynamic file-change-type-enum",
        ),
        wire().replace("(s/conformer file-change-type-enum)", "(s/conformer other)"),
        wire().replace(
            "{1 :created 2 :changed 3 :deleted}",
            "(hash-map 1 :created)",
        ),
        wire().replace(
            "(s/def :file-event/type",
            "(defn int? {:t/sig '[number? :-> boolean?]} [value] true)\n(s/def :file-event/type",
        ),
        format!("{}\n(def file-change-type-enum {{1 :removed}})", wire()),
    ] {
        let report = Session::default().check("coercer.clj", &source);
        assert!(!report.complete, "{report:#?}");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|finding| finding.code == "PNX9001"),
            "{report:#?}"
        );
    }
}

#[test]
fn enum_proposals_do_not_capture_a_table_across_an_intervening_initializer() {
    let selected = [":file-event/type".into()];
    let provider = Source {
        path: "coercer.clj",
        source: wire(),
    };
    let mut session = Session::default();
    let initial = session
        .propose_contracts(&[provider], &[], &selected)
        .unwrap();
    assert!(initial.externs.unwrap().contains("1 :created"));

    let replaced = provider.source.replace(
        "(s/def :file-event/type",
        "(def changed (.bindRoot #'file-change-type-enum {1 :renamed}))\n(s/def :file-event/type",
    );
    let proposal = session
        .propose_contracts(
            &[Source {
                source: &replaced,
                ..provider
            }],
            &[],
            &selected,
        )
        .unwrap();
    assert_eq!(proposal.externs.as_deref(), Some("{}\n"));
}

#[test]
fn hidden_reader_forms_do_not_produce_portable_enums() {
    let source = r#"(ns wire (:require [clojure.spec.alpha :as s]))
(s/def :event/type (s/and int? {1 :open} (s/conformer {1 :open})))"#;
    let mut session = Session::default();
    let selected = [":event/type".into()];
    let propose = |session: &mut Session, source: &str| {
        session
            .propose_contracts(
                &[Source {
                    path: "wire.clj",
                    source,
                }],
                &[],
                &selected,
            )
            .unwrap()
    };
    assert!(
        propose(&mut session, source)
            .externs
            .unwrap()
            .contains("1 :open")
    );

    for guard in [
        "int? #_ #test/rebind nil",
        "^{:probe #test/rebind nil} int?",
    ] {
        let unproven = source.replace("int?", guard);
        let proposal = propose(&mut session, &unproven);
        assert_eq!(proposal.externs.as_deref(), Some("{}\n"));
    }
}

#[test]
fn a_portable_enum_requires_equivalent_literal_tables() {
    let provider = Source {
        path: "enum.edn",
        source: "",
    };
    let client = Source {
        path: "client.clj",
        source: r#"(ns client (:require [clojure.spec.alpha :as s] [wire]))
          (defn created {:t/sig '[:-> #{:created}]} [] (s/conform :event/type 1))"#,
    };
    let check = |predicate: &str, conformer: &str| {
        let source = format!(
            "{{:event/type {{:spec (clojure.spec.alpha/and clojure.core/int? {predicate}\
                (clojure.spec.alpha/conformer {conformer})) :ns wire}}}}"
        );
        Session::default().check_project_with_externs(
            &[client],
            &[Source {
                source: &source,
                ..provider
            }],
        )
    };
    let accepted = check("{1 :created 2 :changed}", "{2 :changed 1 :created}");
    assert!(accepted.ok && accepted.complete, "{accepted:#?}");

    for (predicate, conformer, code) in [
        ("{1 :created}", "{1 :changed}", "PNX9001"),
        ("{1 :created}", "{1 nil}", "PNX9001"),
        ("{1 :created +1 :changed}", "{1 :created}", "PNX9000"),
    ] {
        let rejected = check(predicate, conformer);
        assert!(!rejected.complete, "{rejected:#?}");
        assert!(
            rejected.files.iter().any(|file| file.path == provider.path
                && file.diagnostics.iter().any(|finding| finding.code == code)),
            "{rejected:#?}"
        );
    }
}

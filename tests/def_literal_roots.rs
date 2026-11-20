use parenox::{Session, Source};

const PROVIDER: Source<'_> = Source {
    path: "config.clj",
    source: r#"
(ns demo.root-config)
(def ^:private defaults {:mode :safe
                         :options [nil false]
                         :targets #{:local :remote}
                         :label "mesh"})
(def setup {:layers [defaults {:copy defaults}]
            :quoted '(configure defaults)})
"#,
};

const CONSUMER: Source<'_> = Source {
    path: "consumer.clj",
    source: r#"
(ns demo.root-consumer (:require [demo.root-config :as config]))
(defn mode {:t/sig '[:-> #{:safe}]} [] (get-in config/setup [:layers 1 :copy :mode]))
(defn disabled {:t/sig '[:-> #{false}]} [] (get-in config/setup [:layers 0 :options 1]))
(defn absent {:t/sig '[:-> nil?]} [] (get-in config/setup [:layers 1 :copy :options 0]))
(defn quoted-name {:t/sig '[:-> symbol?]} [] (nth (:quoted config/setup) 1))
(defn independent {:t/sig '[:-> number?]} [] 1)
"#,
};

#[test]
fn captures_exact_nil_and_false_from_adjacent_roots() {
    let source = r#"
(ns demo.roots)
(def ^:private nothing nil)
;; A comment is allowed between the two declarations.
(def missing nothing)
(def disabled false)
(def ^:private settings {:nested [disabled {:again disabled}]})
(defn absent {:t/sig '[:-> nil?]} [] missing)
(defn disabled-copy {:t/sig '[:-> #{false}]} [] (get-in settings [:nested 1 :again]))
"#;
    let report = Session::default().check("roots.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn exports_a_public_value_captured_from_a_private_root() {
    let exported = Session::default().export_externs(&[PROVIDER], &[]);
    let snapshot = exported
        .edn
        .unwrap_or_else(|| panic!("{:#?}", exported.report));
    assert!(snapshot.contains("demo.root-config/setup"), "{snapshot}");
    assert!(
        !snapshot.contains("demo.root-config/defaults"),
        "{snapshot}"
    );
    assert!(snapshot.contains("(configure defaults)"), "{snapshot}");
    assert!(snapshot.contains("#{"), "{snapshot}");

    let externs = Source {
        path: "config.edn",
        source: &snapshot,
    };
    let validation = Session::default().validate_externs(&[PROVIDER], &[externs]);
    assert!(validation.ok && validation.complete, "{validation:#?}");

    let mut client = Session::default();
    let valid = client.check_project_with_externs(&[CONSUMER], &[externs]);
    assert!(valid.ok && valid.complete, "{valid:#?}");
    let incompatible = CONSUMER.source.replace("#{:safe}", "#{:other}");
    let rejected = client.check_project_with_externs(
        &[Source {
            source: &incompatible,
            ..CONSUMER
        }],
        &[externs],
    );
    assert!(!rejected.ok && rejected.complete, "{rejected:#?}");
    let [finding] = rejected.files[0].diagnostics.as_slice() else {
        panic!("{rejected:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some(":safe"));
}

#[test]
fn editing_the_captured_root_rechecks_only_consumers_of_its_value() {
    let mut session = Session::default();
    let before = session.check_project(&[CONSUMER, PROVIDER], None);
    assert!(before.ok && before.complete, "{before:#?}");
    let cached = session.check_project(&[CONSUMER, PROVIDER], None);
    assert_eq!(cached.files[0].stats.reused_functions, 5, "{cached:#?}");

    let changed = PROVIDER.source.replace(":safe", ":fast");
    let after = session.check_project(
        &[
            CONSUMER,
            Source {
                source: &changed,
                ..PROVIDER
            },
        ],
        None,
    );
    assert!(!after.ok && after.complete, "{after:#?}");
    let report = &after.files[0];
    assert_eq!(report.stats.checked_functions, 4, "{after:#?}");
    assert_eq!(report.stats.reused_functions, 1, "{after:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{after:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some(":fast"));

    let restored = session.check_project(&[CONSUMER, PROVIDER], None);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn unsupported_initializer_paths_keep_a_gap_at_the_captured_definition() {
    for (prefix, initializer) in [
        ("(ns root-cases) (def root :ready)", "root-cases/root"),
        (
            "(ns root-cases) (def root :ready) (def unrelated false)",
            "root",
        ),
        (
            "(ns root-cases) (def root :ready) (alter-var-root #'root (constantly :changed))",
            "root",
        ),
        ("(ns root-cases) (def root :ready)", "{root :value}"),
        ("(ns root-cases) (def root :ready)", "#{root}"),
        ("(ns root-cases) (def root :ready)", "(identity root)"),
        ("(ns root-cases) (def root (identity :ready))", "root"),
        ("(ns root-cases) (def ^:dynamic root :ready)", "root"),
        (
            "(ns root-cases) (def ^{:doc (str \"metadata\")} root :ready)",
            "root",
        ),
        ("(ns root-cases) (def root :ready) #_(unknown-form)", "root"),
        ("(def root :ready)", "root"),
    ] {
        let source = format!("{prefix}\n(def snapshot {initializer})");
        let start = source.len() - initializer.len() - 1;
        assert_gap_at_initializer(&source, start);
    }
}

#[test]
fn forward_references_do_not_capture_later_roots() {
    let source = "(ns root-cases)\n(def snapshot root)\n(def root :ready)";
    let start = source.find("(def snapshot root)").unwrap() + "(def snapshot ".len();
    assert_gap_at_initializer(source, start);
}

#[test]
fn a_capture_with_evaluated_metadata_remains_unchecked() {
    let source = r#"
(ns root-cases)
(def root :ready)
(def ^{:doc (str "metadata")} snapshot root)
"#;
    assert_gap_at_initializer(source, source.rfind("root").unwrap());
}

fn assert_gap_at_initializer(source: &str, start: usize) {
    let report = Session::default().check("root-cases.clj", source);
    assert!(!report.complete, "{source}\n{report:#?}");
    assert!(
        report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "PNX9001" && diagnostic.span.start.offset == start
        }),
        "{source}\n{report:#?}"
    );
}

use std::{fs, process::Command};

use parenox::{Session, Source};

const LIBRARY: Source<'_> = Source {
    path: "coercer.clj",
    source: r#"(ns lsp4clj.coercer (:require [clojure.spec.alpha :as s]))
(s/def :server-capabilities/code-lens-provider
  (s/conformer (fn [element] {:resolve-provider element})))
(s/def :markup/kind #{"plaintext" "markdown"})
"#,
};
const WRAP: &str = ":server-capabilities/code-lens-provider";
const KIND: &str = ":markup/kind";
const ACTION: &str = ":server-capabilities/code-action-provider";
const COMMAND: &str = ":server-capabilities/execute-command-provider";
const SIGNATURE_HELP: &str = ":server-capabilities/signature-help-provider";
const SEMANTIC_TOKENS: &str = ":server-capabilities/semantic-tokens-provider";

const CLIENT: Source<'_> = Source {
    path: "consumer.clj",
    source: r#"(ns consumer (:require [clojure.spec.alpha :as s] [lsp4clj.coercer]))
(defn wrap {:t/sig '[:-> #{:clojure.spec.alpha/invalid}]} []
  (:resolve-provider (s/conform :server-capabilities/code-lens-provider ::s/invalid)))
(defn accepts {:t/sig '[:markup/kind :-> :markup/kind]} [kind] kind)
(defn read-kind {:t/sig '[:-> :markup/kind]} [] (accepts "markdown"))
"#,
};

#[test]
fn generated_named_specs_check_consumers_without_source_and_refresh_in_place() {
    let mut session = Session::default();
    let selected = [WRAP.into(), KIND.into()];
    let contract = session
        .propose_contracts(&[LIBRARY], &[], &selected)
        .unwrap();
    let edn = contract
        .externs
        .unwrap_or_else(|| panic!("{:#?}", contract.report));
    let external = Source {
        path: "contracts.edn",
        source: &edn,
    };
    let validation = session.validate_externs(&[LIBRARY], &[external]);
    assert!(validation.ok && validation.complete, "{validation:#?}");

    let report = session.check_project_with_externs(&[CLIENT], &[external]);
    assert!(report.ok && report.complete, "{report:#?}");

    let bad_argument = CLIENT.source.replace("\"markdown\"", "\"html\"");
    let report = session.check_project_with_externs(
        &[Source {
            source: &bad_argument,
            ..CLIENT
        }],
        &[external],
    );
    let [error] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.contract.as_ref().unwrap().name, "consumer/accepts");
    assert!(error.expected.as_deref().unwrap().contains("markdown"));
    assert_eq!(error.actual.as_deref(), Some("\"html\""));

    let baseline = session.check_project_with_externs(&[CLIENT], &[external]);
    assert!(baseline.ok && baseline.complete, "{baseline:#?}");
    let changed = edn.replace(":resolve-provider", ":another-field");
    let report = session.check_project_with_externs(
        &[CLIENT],
        &[Source {
            source: &changed,
            ..external
        }],
    );
    let [error] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.actual.as_deref(), Some("Nil"));
    let restored = session.check_project_with_externs(&[CLIENT], &[external]);
    assert!(restored.ok && restored.complete, "{restored:#?}");

    let missing_require = CLIENT.source.replace(" [lsp4clj.coercer]", "");
    let report = session.check_project_with_externs(
        &[Source {
            source: &missing_require,
            ..CLIENT
        }],
        &[external],
    );
    assert!(!report.complete, "{report:#?}");
    assert!(
        report.files[0]
            .diagnostics
            .iter()
            .any(|d| d.message.contains("registrations are loaded"))
    );
}

#[test]
fn cli_proposals_are_stable_and_leave_existing_contracts_alone() {
    let temp = std::env::temp_dir().join(format!("parenox-spec-proposals-{}", std::process::id()));
    fs::create_dir_all(&temp).unwrap();
    let library = temp.join(LIBRARY.path);
    let manual = temp.join("manual.edn");
    let original = format!(
        "{}\n(s/def :demo/unknown (s/conformer (fn [x] (println x))))\n",
        LIBRARY.source
    );
    let existing = r#"{:markup/kind {:spec #{"plaintext" "markdown"} :ns lsp4clj.coercer}
      lsp4clj.coercer/other [string? :-> string?]}"#;
    fs::write(&library, &original).unwrap();
    fs::write(&manual, existing).unwrap();

    let run = |names: &[&str], only: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
        command
            .arg("propose-contracts")
            .arg(&library)
            .arg("--externs")
            .arg(&manual);
        for name in names {
            command.args(["--spec", name]);
        }
        if only {
            command.arg("--externs-only");
        }
        command.output().unwrap()
    };
    let actual = run(&[WRAP, KIND, ":demo/unknown"], false);
    let reordered = run(&[KIND, ":demo/unknown", WRAP, KIND], false);
    let externs = run(&[WRAP, KIND], true);
    assert_eq!(fs::read_to_string(&library).unwrap(), original);
    assert_eq!(fs::read_to_string(&manual).unwrap(), existing);
    fs::remove_dir_all(temp).unwrap();

    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert!(reordered.status.success() && externs.status.success());
    assert_eq!(actual.stdout, reordered.stdout);
    let report = String::from_utf8(actual.stdout).unwrap();
    assert!(report.contains(":name :demo/unknown") && report.contains("Conformed result"));
    assert!(
        report.contains(":input \"Input (Any)\"") && report.contains(":resolve-provider: Input")
    );
    assert!(report.contains(":form \"(s/def :server-capabilities/code-lens-provider"));
    assert!(report.contains(":extern :existing") && report.contains(":extern :generated"));
    let exported = String::from_utf8(externs.stdout).unwrap();
    assert!(exported.contains(WRAP) && !exported.contains(KIND));
    assert!(!exported.contains("lsp4clj.coercer/other"));
}

#[test]
fn proposals_require_evidence_of_a_standard_spec_and_its_registration_namespace() {
    let replacement = Source {
        path: "clojure/spec/alpha.clj",
        source: "(ns clojure.spec.alpha) (defmacro def [_ _] nil)",
    };
    let script = Source {
        path: "script.clj",
        source: "(clojure.spec.alpha/def :markup/kind clojure.core/string?)",
    };
    for (sources, reason) in [
        (&[LIBRARY, replacement][..], "project's clojure.spec.alpha"),
        (&[script][..], "Standalone registration"),
    ] {
        let proposals = Session::default()
            .propose_contracts(sources, &[], &[KIND.into()])
            .unwrap();
        assert_eq!(proposals.externs.as_deref(), Some("{}\n"));
        let edn = proposals
            .edn
            .unwrap_or_else(|| panic!("{:#?}", proposals.report));
        assert!(edn.contains(reason), "{edn}");
    }
}

#[test]
#[ignore = "requires the pinned lsp4clj 2.0.1 coercer source"]
fn pinned_lsp4clj_proposals_check_a_consumer_without_library_sources() {
    let path = std::env::var("PARENOX_LSP4CLJ_COERCER").expect("set PARENOX_LSP4CLJ_COERCER");
    let source = fs::read_to_string(&path).unwrap();
    let selected = [WRAP, KIND, ACTION, COMMAND, SIGNATURE_HELP, SEMANTIC_TOKENS].map(String::from);
    let mut session = Session::default();
    let proposals = session
        .propose_contracts(
            &[Source {
                path: &path,
                source: &source,
            }],
            &[],
            &selected,
        )
        .unwrap();
    let edn = proposals
        .externs
        .unwrap_or_else(|| panic!("{:#?}", proposals.report));
    assert!(edn.contains(SEMANTIC_TOKENS));
    assert!(
        proposals
            .edn
            .as_ref()
            .unwrap()
            .contains("\"Exception classes from custom lookups\"")
    );
    let externs = [Source {
        path: "lsp4clj.edn",
        source: &edn,
    }];
    let client = format!(
        "{}{}",
        CLIENT.source,
        r#"(defn action {:t/sig '[:-> #{:rename}]} []
  (first (:code-action-kinds (s/conform :server-capabilities/code-action-provider [:rename]))))
(defn commands {:t/sig '[:-> nil?]} []
  (s/conform :server-capabilities/execute-command-provider ::s/invalid))
(defn signature-map {:t/sig '[:-> {:work-done-progress #{true}}]} []
  (s/conform :server-capabilities/signature-help-provider {:work-done-progress true}))
(defn signature-vector {:t/sig '[:-> {:trigger-characters (s/tuple #{"("})}]} []
  (s/conform :server-capabilities/signature-help-provider ["("]))
(defn semantic-tokens
  {:t/sig '[:-> {:legend {:token-types #{:type} :token-modifiers #{:modifier}}
                 :range nil? :full #{false}}]} []
  (s/conform :server-capabilities/semantic-tokens-provider
             {:token-types :type :token-modifiers :modifier}))
"#
    );
    let check = |source| {
        Session::default().check_project_with_externs(&[Source { source, ..CLIENT }], &externs)
    };
    let accepted = check(&client);
    assert!(accepted.ok && accepted.complete, "{accepted:#?}");
    let changed = client.replace("(accepts \"markdown\")", "(accepts \"html\")");
    let rejected = check(&changed);
    let [finding] = rejected.files[0].diagnostics.as_slice() else {
        panic!("{rejected:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert!(finding.message.contains("Argument 1") && finding.message.contains("\"html\""));

    let unchecked =
        format!("{client}\n(defn improper {{:t/sig '[{SEMANTIC_TOKENS} :-> nil?]}} [_] nil)");
    let rejected = check(&unchecked);
    let [finding] = rejected.files[0].diagnostics.as_slice() else {
        panic!("{rejected:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
    assert!(finding.message.contains("unchecked input type"));

    let project_core = Source {
        path: "clojure/core.clj",
        source: "(ns clojure.core)",
    };
    let consumer = Source {
        source: r#"(ns consumer (:require [clojure.spec.alpha :as s] [lsp4clj.coercer]))
                   (defn result {:t/sig '[:-> nil?]} []
                     (s/conform :server-capabilities/semantic-tokens-provider nil))"#,
        ..CLIENT
    };
    let shadowed =
        Session::default().check_project_with_externs(&[consumer, project_core], &externs);
    assert!(
        shadowed.files[0].diagnostics.iter().any(|finding| {
            finding.code == "PNX9001" && finding.message.contains("supported conversion")
        }),
        "{shadowed:#?}"
    );
}

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
const TEXT_SYNC: &str = ":server-capabilities/text-document-sync";
const CAPABILITIES: &str = ":lsp4clj.coercer/server-capabilities";
const POSITION: &str = ":lsp4clj.coercer/position";
const RANGE: &str = ":lsp4clj.coercer/range";
const ADDITIONAL_EDITS: &str = ":lsp4clj.coercer/additional-text-edits";
const DOCUMENT_EDIT: &str = ":lsp4clj.coercer/text-document-edit";

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
    let selected = [
        KIND,
        CAPABILITIES,
        POSITION,
        RANGE,
        ADDITIONAL_EDITS,
        DOCUMENT_EDIT,
    ]
    .map(String::from);
    let manual = Source {
        path: "manual.edn",
        source: r#"{:server-capabilities/code-lens-provider
          {:spec (clojure.spec.alpha/conformer (clojure.core/fn [input] {:resolve-provider input}))
           :ns lsp4clj.coercer}}"#,
    };
    let mut session = Session::default();
    let proposals = session
        .propose_contracts(
            &[Source {
                path: &path,
                source: &source,
            }],
            &[manual],
            &selected,
        )
        .unwrap();
    let edn = proposals
        .externs
        .unwrap_or_else(|| panic!("{:#?}", proposals.report));
    assert!(edn.contains(SEMANTIC_TOKENS));
    assert!(edn.contains(TEXT_SYNC));
    for name in [
        POSITION,
        RANGE,
        ":lsp4clj.coercer/line",
        ":lsp4clj.coercer/character",
        ":lsp4clj.coercer/text-edit",
        ":lsp4clj.coercer/new-text",
        ":lsp4clj.coercer/text-document",
        ":lsp4clj.coercer/version",
        ":lsp4clj.coercer/uri",
    ] {
        assert!(edn.contains(&format!("{name} {{:spec")), "{edn}");
    }
    assert!(
        edn.contains(&format!(
            "{ADDITIONAL_EDITS} {{:spec (clojure.spec.alpha/coll-of :lsp4clj.coercer/text-edit) :ns lsp4clj.coercer}}"
        )),
        "{edn}"
    );
    assert!(
        edn.contains(&format!(
            "{DOCUMENT_EDIT} {{:spec (clojure.spec.alpha/keys :req-un [:lsp4clj.coercer/edits :lsp4clj.coercer/text-document]) :ns lsp4clj.coercer}}"
        )),
        "{edn}"
    );
    assert!(
        edn.contains(
            ":lsp4clj.coercer/edits {:spec (clojure.spec.alpha/coll-of :lsp4clj.coercer/text-edit) :ns lsp4clj.coercer}"
        ),
        "{edn}"
    );
    for alias in [":lsp4clj.coercer/start", ":lsp4clj.coercer/end"] {
        let registration = format!("{alias} {{:spec {POSITION} :ns lsp4clj.coercer}}");
        assert!(edn.contains(&registration), "{edn}");
    }
    let range_name = format!(":name {RANGE} ");
    let range_proposal = proposals
        .edn
        .as_deref()
        .unwrap()
        .lines()
        .find(|line| line.contains(&range_name))
        .expect("range proposal");
    let (_, input) = range_proposal.split_once(":input \"").unwrap();
    let (input, _) = input.split_once('"').unwrap();
    for endpoint in [":start", ":end"] {
        assert!(
            input.contains(&format!("{endpoint}: {{:character: Int32, :line: Int32")),
            "{range_proposal}"
        );
    }
    assert!(
        !edn.contains(":server-capabilities/code-lens-provider {:spec"),
        "{edn}"
    );
    for child in [
        WRAP,
        ACTION,
        COMMAND,
        SIGNATURE_HELP,
        ":server-capabilities/rename-provider",
    ] {
        assert!(edn.contains(child), "{edn}");
    }
    let externs = [
        Source {
            path: "lsp4clj.edn",
            source: &edn,
        },
        manual,
    ];
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
(defn text-sync {:t/sig '[:-> {:open-close #{true} :change t/any :save {:include-text #{true}}}]} []
  (s/conform :server-capabilities/text-document-sync :incremental))
(defn bundle
  {:t/sig '[:-> {:semantic-tokens-provider nil?
                 :code-lens-provider {:resolve-provider #{false}}
                 :server-capabilities/code-lens-provider {:resolve-provider #{::s/invalid}}}]} []
  (s/conform :lsp4clj.coercer/server-capabilities
             {:semantic-tokens-provider false
              :code-lens-provider false
              :server-capabilities/code-lens-provider ::s/invalid}))
(defn valid-bundle {:t/sig '[:-> #{true}]} []
  (s/valid? :lsp4clj.coercer/server-capabilities {:semantic-tokens-provider nil}))
(defn position {:t/sig '[:-> {:line int? :character int? :tag #{:unchanged}}]} []
  (s/conform :lsp4clj.coercer/position
             {:line -2147483648N :character 2147483647N :tag :unchanged}))
(defn invalid-position
  {:t/sig '[:-> (s/tuple #{::s/invalid} #{false} #{::s/invalid} #{false})]} []
  [(s/conform :lsp4clj.coercer/position nil)
   (s/valid? :lsp4clj.coercer/position false)
   (s/conform :lsp4clj.coercer/position {:line nil :character 0})
   (s/valid? :lsp4clj.coercer/position {:line 0 :character false})])
(defn missing-character {:t/sig '[:-> #{::s/invalid}]} []
  (try (s/conform :lsp4clj.coercer/position {:line 2147483648})
       (catch ArithmeticException _ :unreachable)))
(defn overflowing-position
  {:t/sig '[:-> (s/tuple #{:below-min} #{:above-max} #{:above-long-max})]} []
  [(try (s/valid? :lsp4clj.coercer/position {:line -2147483649 :character 0})
        (catch ArithmeticException _ :below-min))
   (try (s/conform :lsp4clj.coercer/position {:line 0 :character 2147483648})
        (catch ArithmeticException _ :above-max))
   (try (s/conform :lsp4clj.coercer/position {:line 9223372036854775808N :character 0})
        (catch IllegalArgumentException _ :above-long-max))])
(defn accepts-position {:t/sig '[:lsp4clj.coercer/position :-> nil?]} [_] nil)
(defn pass-position {:t/sig '[:-> nil?]} []
  (accepts-position {:line -2147483648 :character 2147483647}))
(defn conform-range
  {:t/sig '[:-> {:start {:line int? :character int? :tag #{:keep}}
                 :end {:line int? :character int?}}]} []
  (s/conform :lsp4clj.coercer/range
             {:start {:line -2147483648N :character 2N :tag :keep}
              :end {:line 3N :character 2147483647N}}))
(defn validate-range {:t/sig '[:-> #{true}]} []
  (s/valid? :lsp4clj.coercer/range
            {:start {:line 0 :character 0} :end {:line 1 :character 1}}))
(defn incomplete-range {:t/sig '[:-> (s/tuple #{::s/invalid} #{false})]} []
  [(try (s/conform :lsp4clj.coercer/range
                   {:start {:line 2147483648} :end {:line 0 :character 0}})
        (catch Throwable _ :unexpected))
   (try (s/valid? :lsp4clj.coercer/range
                  {:start {:line 0 :character 0} :end {:character -2147483649}})
        (catch Throwable _ :unexpected))])
(defn overflowing-range
  {:t/sig '[:-> (s/tuple #{:int-overflow} #{:long-overflow} #{:int-overflow} #{:long-overflow})]} []
  [(try (s/conform :lsp4clj.coercer/range
                   {:start {:line -2147483649 :character 0} :end {:line 0 :character 0}})
        (catch ArithmeticException _ :int-overflow)
        (catch Throwable _ :unexpected))
   (try (s/valid? :lsp4clj.coercer/range
                  {:start {:line -9223372036854775809N :character 0} :end {:line 0 :character 0}})
        (catch IllegalArgumentException _ :long-overflow)
        (catch Throwable _ :unexpected))
   (try (s/valid? :lsp4clj.coercer/range
                  {:start {:line 0 :character 0} :end {:line 0 :character 2147483648}})
        (catch ArithmeticException _ :int-overflow)
        (catch Throwable _ :unexpected))
   (try (s/conform :lsp4clj.coercer/range
                   {:start {:line 0 :character 0} :end {:line 0 :character 9223372036854775808N}})
        (catch IllegalArgumentException _ :long-overflow)
        (catch Throwable _ :unexpected))])
(defn accepts-range {:t/sig '[:lsp4clj.coercer/range :-> nil?]} [_] nil)
(defn pass-range {:t/sig '[:-> nil?]} []
  (accepts-range {:start {:line 0 :character 0} :end {:line 1 :character 42}}))
(defn conform-edits
  {:t/sig '[:-> (s/tuple {:new-text #{"first"}
                         :range {:start {:line int? :character int?}
                                 :end {:line int? :character int?}}}
                        {:new-text #{"second"}
                         :range {:start {:line int? :character int?}
                                 :end {:line int? :character int?}}})]} []
  (s/conform :lsp4clj.coercer/additional-text-edits
             [{:new-text "first"
               :range {:start {:line -1N :character 0} :end {:line 1 :character 2}}}
              {:new-text "second"
               :range {:start {:line 3 :character 4} :end {:line 5 :character 6N}}}]))
(defn empty-edits {:t/sig '[:-> (s/tuple #{true} #{true} #{true})]} []
  (let [edits (s/conform :lsp4clj.coercer/additional-text-edits [])]
    [(s/valid? :lsp4clj.coercer/additional-text-edits [])
     (vector? edits)
     (empty? edits)]))
(defn valid-edits {:t/sig '[:-> #{true}]} []
  (s/valid? :lsp4clj.coercer/additional-text-edits
            [{:new-text "x"
              :range {:start {:line 0 :character 0} :end {:line 1 :character 1}}}]))
(defn non-collections {:t/sig '[:-> (s/tuple #{::s/invalid} #{false})]} []
  [(s/conform :lsp4clj.coercer/additional-text-edits nil)
   (s/valid? :lsp4clj.coercer/additional-text-edits false)])
(defn ordered-edits
  {:t/sig '[:-> (s/tuple #{::s/invalid} #{false} #{:overflow})]} []
  (let [wide {:new-text ""
              :range {:start {:line 2147483648 :character 0} :end {:line 1 :character 1}}}]
    [(try (s/conform :lsp4clj.coercer/additional-text-edits [{} wide])
          (catch Throwable _ :unexpected))
     (try (s/valid? :lsp4clj.coercer/additional-text-edits [{} wide])
          (catch Throwable _ :unexpected))
     (try (s/conform :lsp4clj.coercer/additional-text-edits [wide {}])
          (catch ArithmeticException _ :overflow)
          (catch Throwable _ :unexpected))]))
(defn accepts-edits {:t/sig '[:lsp4clj.coercer/additional-text-edits :-> nil?]} [_] nil)
(defn pass-edits {:t/sig '[:-> nil?]} []
  (accepts-edits [{:new-text "argument"
                   :range {:start {:line 0 :character 0} :end {:line 1 :character 1}}}]))
(defn conform-document-edit
  {:t/sig '[:-> {:text-document {:version int? :uri #{"file:///edited"}}
                 :edits (s/tuple {:new-text #{"update"}
                                   :range {:start {:line int? :character int?}
                                           :end {:line int? :character int?}}})}]} []
  (s/conform :lsp4clj.coercer/text-document-edit
             {:text-document {:version 7N :uri "file:///edited"}
              :edits [{:new-text "update"
                       :range {:start {:line -1N :character 0}
                               :end {:line 1 :character 2N}}}]}))
(defn valid-document-edit {:t/sig '[:-> #{true}]} []
  (s/valid? :lsp4clj.coercer/text-document-edit
            {:text-document {:version 0 :uri "file:///edited"}
             :edits [{:new-text "update"
                      :range {:start {:line 0 :character 0} :end {:line 1 :character 0}}}]}))
(defn empty-document-edit {:t/sig '[:-> (s/tuple #{true} #{true} #{true})]} []
  (let [input {:text-document {:version 0 :uri "file:///edited"} :edits []}
        edits (:edits (s/conform :lsp4clj.coercer/text-document-edit input))]
    [(s/valid? :lsp4clj.coercer/text-document-edit input)
     (vector? edits)
     (empty? edits)]))
(defn missing-document-fields
  {:t/sig '[:-> (s/tuple #{::s/invalid} #{false} #{::s/invalid} #{false})]} []
  (let [wide {:start {:line 2147483648 :character 0} :end {:line 0 :character 1}}]
    [(try (s/conform :lsp4clj.coercer/text-document-edit
                     {:edits [{:new-text "update" :range wide}]})
          (catch Throwable _ :unexpected))
     (try (s/valid? :lsp4clj.coercer/text-document-edit
                    {:text-document {:version 2147483648 :uri "file:///edited"}})
          (catch Throwable _ :unexpected))
     (try (s/conform :lsp4clj.coercer/text-document-edit
                     {:text-document {:version 2147483648} :edits []})
          (catch Throwable _ :unexpected))
     (try (s/valid? :lsp4clj.coercer/text-document-edit
                    {:text-document {:version 0 :uri "file:///edited"} :edits [{:range wide}]})
          (catch Throwable _ :unexpected))]))
(defn accepts-document-edit {:t/sig '[:lsp4clj.coercer/text-document-edit :-> nil?]} [_] nil)
(defn pass-document-edit {:t/sig '[:-> nil?]} []
  (accepts-document-edit
    {:text-document {:version 0 :uri "file:///edited"}
     :edits [{:new-text "update"
              :range {:start {:line 0 :character 0} :end {:line 1 :character 77}}}]}))
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

    let changed = client.replace(":character 2147483647})", ":character false})");
    let rejected = check(&changed);
    let [finding] = rejected.files[0].diagnostics.as_slice() else {
        panic!("{rejected:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert!(
        finding.message.contains("Argument 1") && finding.message.contains(":character: false")
    );

    let changed = client.replace(":character 77", ":character false");
    let rejected = check(&changed);
    let [finding] = rejected.files[0].diagnostics.as_slice() else {
        panic!("{rejected:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        finding.contract.as_ref().unwrap().name,
        "consumer/accepts-document-edit"
    );
    assert!(
        finding.message.contains("Argument 1")
            && finding.message.contains(":edits:")
            && finding.message.contains(":character: false")
    );

    let changed = client.replace(":new-text \"argument\"", ":new-text false");
    let rejected = check(&changed);
    let [finding] = rejected.files[0].diagnostics.as_slice() else {
        panic!("{rejected:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        finding.contract.as_ref().unwrap().name,
        "consumer/accepts-edits"
    );
    assert!(finding.message.contains("Argument 1") && finding.message.contains(":new-text: false"));

    let changed = client.replace(":character 42}})", ":character false}})");
    let rejected = check(&changed);
    let [finding] = rejected.files[0].diagnostics.as_slice() else {
        panic!("{rejected:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        finding.contract.as_ref().unwrap().name,
        "consumer/accepts-range"
    );
    assert!(
        finding.message.contains("Argument 1") && finding.message.contains(":character: false")
    );

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

#[test]
#[ignore = "requires the pinned lsp4clj 2.0.1 coercer source"]
fn pinned_lsp4clj_proposals_export_changes_for_cli_consumers() {
    let source = fs::canonicalize(
        std::env::var_os("PARENOX_LSP4CLJ_COERCER").expect("set PARENOX_LSP4CLJ_COERCER"),
    )
    .unwrap();
    let temp = std::env::temp_dir().join(format!("parenox-changes-{}", std::process::id()));
    fs::create_dir_all(&temp).unwrap();
    let manual = temp.join("manual.edn");
    let generated = temp.join("generated.edn");
    let consumer = temp.join("consumer.clj");
    let manual_edn = r#"{:lsp4clj.coercer/line
      {:spec (clojure.spec.alpha/and clojure.core/integer?
               (clojure.spec.alpha/conformer clojure.core/int))
       :ns lsp4clj.coercer}}"#;
    fs::write(&manual, manual_edn).unwrap();

    let client = r#"(ns consumer (:require [clojure.spec.alpha :as s] [lsp4clj.coercer :as wire]))
(defn projected
  {:t/sig '[:-> (s/tuple int? #{"update"} #{true} #{true})]} []
  (let [changes (s/conform ::wire/changes
                  {"file:///first" [{:new-text "update"
                                     :range {:start {:line -2N :character 0}
                                             :end {:line 0 :character 1}}}]
                   "file:///empty" []})
        empty-edits (get changes "file:///empty")]
    [(get-in changes ["file:///first" 0 :range :start :line])
     (get-in changes ["file:///first" 0 :new-text])
     (vector? empty-edits)
     (empty? empty-edits)]))
(defn empty-changes {:t/sig '[:-> (s/tuple {} #{true})]} []
  [(s/conform ::wire/changes {}) (s/valid? ::wire/changes {})])
(defn rejects-key-before-value {:t/sig '[:-> (s/tuple #{::s/invalid} #{false})]} []
  (let [input {:invalid-uri [{:new-text "wide"
                             :range {:start {:line 2147483648 :character 0}
                                     :end {:line 0 :character 1}}}]}]
    [(try (s/conform ::wire/changes input) (catch Throwable _ :unexpected))
     (try (s/valid? ::wire/changes input) (catch Throwable _ :unexpected))]))
(defn accepts-changes {:t/sig '[::wire/changes :-> nil?]} [_] nil)
(defn pass-changes {:t/sig '[:-> nil?]} []
  (accepts-changes
   {"file:///argument" [{:new-text "update"
                         :range {:start {:line 0 :character 0}
                                 :end {:line 0 :character 88}}}]}))
"#;
    let invalid = client.replace(":character 88", ":character false");
    let opaque = format!(
        r#"{client}
(defn opaque-changes
  {{:t/sig '[{{"file:///first" (s/tuple ::wire/text-edit)}} :-> boolean?]}} [input]
  (s/valid? ::wire/changes input))"#
    );

    for split in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
        command
            .current_dir(&temp)
            .arg("propose-contracts")
            .arg(&source)
            .args(["--spec", ":lsp4clj.coercer/changes", "--externs-only"]);
        if split {
            command.arg("--externs").arg(&manual);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let edn = String::from_utf8(output.stdout).unwrap();
        for name in [
            ":lsp4clj.coercer/changes",
            ":lsp4clj.coercer/edits",
            ":lsp4clj.coercer/text-edit",
        ] {
            assert!(edn.contains(&format!("{name} {{:spec")), "{edn}");
        }
        assert_eq!(
            edn.contains(":lsp4clj.coercer/line {:spec"),
            !split,
            "{edn}"
        );
        assert_eq!(fs::read_to_string(&manual).unwrap(), manual_edn);
        fs::write(&generated, edn).unwrap();

        let check = |text: &str| {
            fs::write(&consumer, text).unwrap();
            let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
            command
                .current_dir(&temp)
                .arg("check")
                .arg(&consumer)
                .args(["--format", "json"])
                .arg("--externs")
                .arg(&generated);
            if split {
                command.arg("--externs").arg(&manual);
            }
            let output = command.output().unwrap();
            let report: serde_json::Value = serde_json::from_slice(&output.stdout)
                .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
            (output.status.code(), report)
        };
        let (status, accepted) = check(client);
        assert_eq!(status, Some(0), "{accepted:#}");
        assert_eq!(accepted["complete"], true, "{accepted:#}");

        for (text, code, detail) in [
            (invalid.as_str(), "PNX2322", ":character: false"),
            (opaque.as_str(), "PNX9001", "literal map"),
        ] {
            let (status, rejected) = check(text);
            assert_eq!(status, Some(1), "{rejected:#}");
            let [finding] = rejected["files"][0]["diagnostics"]
                .as_array()
                .unwrap()
                .as_slice()
            else {
                panic!("{rejected:#}");
            };
            assert_eq!(finding["code"], code, "{finding:#}");
            assert!(
                finding["message"].as_str().unwrap().contains(detail),
                "{finding:#}"
            );
        }
    }
    fs::remove_dir_all(temp).unwrap();
}

#[test]
#[ignore = "requires the pinned lsp4clj 2.0.1 coercer source"]
fn pinned_lsp4clj_proposals_export_file_event_type_for_cli_consumers() {
    let source = fs::canonicalize(
        std::env::var_os("PARENOX_LSP4CLJ_COERCER").expect("set PARENOX_LSP4CLJ_COERCER"),
    )
    .unwrap();
    let temp = std::env::temp_dir().join(format!("parenox-file-event-{}", std::process::id()));
    fs::create_dir_all(&temp).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_parenox"))
            .current_dir(&temp)
            .args(args)
            .output()
            .unwrap()
    };
    let source = source.to_str().unwrap();
    let propose = |extra: &[&str]| {
        let mut args = vec!["propose-contracts", source, "--spec", ":file-event/type"];
        args.extend_from_slice(extra);
        let output = run(&args);
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap()
    };
    let report = propose(&[]);
    assert_eq!(report, propose(&["--spec", ":file-event/type"]));
    assert!(report.contains(":input \"1 | 2 | 3\""), "{report}");
    assert!(report.contains(":extern :generated"), "{report}");

    let exported = propose(&["--externs-only"]);
    let forms = parenox::reader::read(&exported).unwrap();
    let [name, declaration] = forms[0].map().unwrap() else {
        panic!("{exported}");
    };
    assert_eq!(forms.len(), 1, "{exported}");
    assert_eq!(name.keyword(), Some(":file-event/type"));
    let spec = declaration
        .map()
        .unwrap()
        .as_chunks::<2>()
        .0
        .iter()
        .find(|pair| pair[0].keyword() == Some(":spec"))
        .map(|pair| &pair[1].span)
        .unwrap();
    assert_eq!(
        &exported[spec.start.offset..spec.end.offset],
        "(clojure.spec.alpha/and clojure.core/int? {1 :created 2 :changed 3 :deleted} \
         (clojure.spec.alpha/conformer {1 :created 2 :changed 3 :deleted}))"
    );

    fs::write(temp.join("generated.edn"), &exported).unwrap();
    let pinned_source = fs::read_to_string(source).unwrap();
    let (leaf_source, _) = pinned_source.split_once("(s/def ::file-event ").unwrap();
    fs::write(temp.join("provider.clj"), leaf_source).unwrap();
    let validated = run(&[
        "validate-externs",
        "provider.clj",
        "--externs",
        "generated.edn",
        "--format",
        "json",
    ]);
    assert!(validated.status.success(), "{validated:?}");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&validated.stdout).unwrap()["complete"],
        true
    );
    fs::remove_file(temp.join("provider.clj")).unwrap();
    let existing = propose(&["--externs", "generated.edn"]);
    assert!(existing.contains(":extern :existing"), "{existing}");
    assert_eq!(
        propose(&["--externs", "generated.edn", "--externs-only"]),
        "{}\n"
    );
    assert_eq!(
        fs::read_to_string(temp.join("generated.edn")).unwrap(),
        exported
    );

    let client = r#"(ns consumer (:require [clojure.spec.alpha :as s] [lsp4clj.coercer]))
(defn events
  {:t/sig '[:-> (s/tuple #{:created} #{:changed} #{:deleted} #{::s/invalid} #{::s/invalid})]} []
  [(s/conform :file-event/type 1) (s/conform :file-event/type 2) (s/conform :file-event/type 3)
   (s/conform :file-event/type 4) (s/conform :file-event/type 1N)])
(defn classify {:t/sig '[int? :-> #{:created :changed :deleted ::s/invalid}]} [x]
  (s/conform :file-event/type x))
(defn validates {:t/sig '[:-> (s/tuple #{true} #{false})]} []
  [(s/valid? :file-event/type 2) (s/valid? :file-event/type 4)])
(defn accepts {:t/sig '[:file-event/type :-> nil?]} [_] nil)
(defn pick {:t/sig '[:-> nil?]} [] (accepts 2))
"#;
    let check = |input: &str, status: i32, complete: bool| {
        fs::write(temp.join("consumer.clj"), input).unwrap();
        let output = run(&[
            "check",
            "consumer.clj",
            "--externs",
            "generated.edn",
            "--format",
            "json",
        ]);
        let report: serde_json::Value =
            serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{output:?}"));
        assert_eq!(output.status.code(), Some(status), "{report:#}");
        assert_eq!(report["complete"], complete, "{report:#}");
        report
    };
    check(client, 0, true);
    for (arg, actual) in [
        ("4", "4"),
        ("1N", "Arbitrary-precision integer with value 1"),
    ] {
        let rejected = check(
            &client.replace("(accepts 2)", &format!("(accepts {arg})")),
            1,
            true,
        );
        let diagnostics = rejected["files"][0]["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 1, "{rejected:#}");
        assert_eq!(diagnostics[0]["code"], "PNX2322", "{rejected:#}");
        assert_eq!(diagnostics[0]["actual"], actual, "{rejected:#}");
    }
    let narrowed = check(
        &client.replace(":created :changed :deleted ::s/invalid", ":created"),
        1,
        true,
    );
    let [diagnostic] = narrowed["files"][0]["diagnostics"]
        .as_array()
        .unwrap()
        .as_slice()
    else {
        panic!("{narrowed:#}");
    };
    assert_eq!(diagnostic["code"], "PNX2322", "{narrowed:#}");
    let domain = diagnostic["actual"].as_str().unwrap();
    for result in [
        ":created",
        ":changed",
        ":deleted",
        ":clojure.spec.alpha/invalid",
    ] {
        assert!(domain.contains(result), "{narrowed:#}");
    }
    let untrusted = check(&client.replace(" [lsp4clj.coercer]", ""), 1, false);
    let diagnostics = untrusted["files"][0]["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["code"] == "PNX9001"
                && diagnostic["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("registrations are loaded"))
        }),
        "{untrusted:#}"
    );
    fs::remove_dir_all(temp).unwrap();
}

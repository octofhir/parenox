use parenox_analysis::{Report, Session};

const SOURCE: &str = r#"
(ns demo.semantic (:require [clojure.spec.alpha :as s]))
(s/def :server-capabilities/semantic-tokens-provider
  (s/conformer #(when (and (:token-types %) (:token-modifiers %))
                  {:legend {:token-types (:token-types %)
                            :token-modifiers (:token-modifiers %)}
                   :range (:range %)
                   :full (boolean (get % :full false))})))
(s/def ::capabilities (s/keys :opt-un [:server-capabilities/semantic-tokens-provider]))
"#;

fn check(source: &str) -> Report {
    Session::default().check("demo/semantic.clj", source)
}

#[test]
fn literal_guards_narrow_repeated_reads_and_preserve_required_output_fields() {
    let body = r#"
(defn detailed {:t/sig '[:-> {:legend {:token-types #{::s/invalid}
                                      :token-modifiers (s/coll-of keyword? :kind vector?)}
                             :range #{false} :full #{true}}]} []
  (let [input {:token-types ::s/invalid :token-modifiers [] :range false :full :enabled}]
    (try (s/conform :server-capabilities/semantic-tokens-provider input)
         (catch Throwable _ :unexpected))))
(defn defaults {:t/sig '[:-> {:legend {:token-types #{""} :token-modifiers int?}
                             :range nil? :full #{false}}]} []
  (s/conform :server-capabilities/semantic-tokens-provider {:token-types "" :token-modifiers 0}))
(defn conditional
  {:t/sig '[boolean? :-> (s/nilable {:legend {:token-types #{true} :token-modifiers #{:enabled}}
                                      :range nil? :full #{false}})]}
  [flag]
  (s/conform :server-capabilities/semantic-tokens-provider
             {:token-types flag :token-modifiers :enabled}))
(defn skipped {:t/sig '[:-> (s/tuple nil? nil? nil? #{true})]} []
  [(s/conform :server-capabilities/semantic-tokens-provider {:token-types []})
   (s/conform :server-capabilities/semantic-tokens-provider {:token-types false :token-modifiers []})
   (s/conform :server-capabilities/semantic-tokens-provider ::s/invalid)
   (try (s/valid? :server-capabilities/semantic-tokens-provider nil)
        (catch Throwable _ false))])
(defn nested {:t/sig '[:-> {:semantic-tokens-provider nil?}]} []
  (s/conform ::capabilities {:semantic-tokens-provider false}))
"#;
    let report = check(&format!("{SOURCE}{body}"));
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn opaque_lookups_can_throw_and_the_legend_reads_are_unrestricted() {
    let body = r#"
(defn broad
  {:t/sig '[map? :-> (s/nilable {:legend {:token-types t/any :token-modifiers t/any}
                                  :range t/any :full boolean?})]}
  [input]
  (s/conform :server-capabilities/semantic-tokens-provider input))
(defn normal {:t/sig '[t/any :-> #{true}]} [input]
  (s/valid? :server-capabilities/semantic-tokens-provider input))
"#;
    let report = check(&format!("{SOURCE}{body}"));
    assert!(report.ok && report.complete, "{report:#?}");

    let unexpected = r#"
(defn caught {:t/sig '[map? :-> #{true}]} [input]
  (try (s/valid? :server-capabilities/semantic-tokens-provider input)
       (catch Throwable _ :caught)))
"#;
    let report = check(&format!("{SOURCE}{unexpected}"));
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert!(finding.actual.as_deref().unwrap().contains(":caught"));
}

#[test]
fn unknown_input_domains_and_shadowed_lookups_report_gaps() {
    let source = format!(
        "{SOURCE}\n(defn check {{:t/sig '[:server-capabilities/semantic-tokens-provider :-> nil?]}} [_] nil)"
    );
    let report = check(&source);
    assert!(
        !report.complete
            && report.diagnostics.iter().any(|finding| {
                finding.code == "PNX9001" && finding.message.contains("unchecked input type")
            }),
        "{report:#?}"
    );

    let shadow = SOURCE.replace(
        "(s/def :server-capabilities",
        "(defn get [& _] false)\n(s/def :server-capabilities",
    );
    let report = check(&shadow);
    assert!(
        !report.complete
            && report.diagnostics.iter().any(|finding| {
                finding.code == "PNX9001" && finding.message.contains("s/conformer")
            }),
        "{report:#?}"
    );
}

use parenox_analysis::{Report, Session};

const SPEC: &str = r#"
(ns demo.signature-help (:require [clojure.spec.alpha :as s]))
(s/def :server-capabilities/signature-help-provider
  (s/conformer #(cond (vector? %) {:trigger-characters %}
                      (map? %) %
                      :else {:trigger-characters %})))
(s/def ::capabilities (s/keys :opt-un [:server-capabilities/signature-help-provider]))
"#;

fn check(source: &str) -> Report {
    Session::default().check("demo/signature_help.clj", source)
}

#[test]
fn the_original_map_or_the_wrapped_input_retains_its_exact_fields() {
    let body = r#"
(defn map-input {:t/sig '[:-> {:work-done-progress #{true}}]} []
  (let [options {:work-done-progress true}]
    (s/conform :server-capabilities/signature-help-provider options)))
(defn vector-input {:t/sig '[:-> {:trigger-characters (s/tuple #{"("})}]} []
  (s/conform :server-capabilities/signature-help-provider ["("]))
(defn entry-input {:t/sig '[:-> #{:trigger}]} []
  (key (:trigger-characters
        (s/conform :server-capabilities/signature-help-provider
                   (clojure.lang.MapEntry/create :trigger "x")))))
(defn nil-input {:t/sig '[:-> {:trigger-characters nil?}]} []
  (s/conform :server-capabilities/signature-help-provider nil))
(defn nested {:t/sig '[:-> {:signature-help-provider {:trigger-characters #{false}}}]} []
  (s/conform ::capabilities {:signature-help-provider false}))
(defn sentinel {:t/sig '[:-> {:trigger-characters #{::s/invalid}}]} []
  (s/conform :server-capabilities/signature-help-provider ::s/invalid))
(defn always-valid {:t/sig '[t/any :-> #{true}]} [input]
  (s/valid? :server-capabilities/signature-help-provider input))
"#;
    let report = check(&format!("{SPEC}{body}"));
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn a_map_type_without_literal_evidence_can_also_be_a_vector() {
    let body = r#"
(defn both {:t/sig '[{:tag keyword?}
                  :-> (s/or :same {:tag keyword?}
                            :wrapped {:trigger-characters {:tag keyword?}})]} [input]
  (s/conform :server-capabilities/signature-help-provider input))
(defn always-map {:t/sig '[t/any :-> map?]} [input]
  (s/conform :server-capabilities/signature-help-provider input))
(defn skipped-map {:t/sig '[(s/nilable string?) :-> map?]} [input]
  (s/conform :server-capabilities/signature-help-provider
             (some-> input (as-> _ {:tag :x}))))
"#;
    let report = check(&format!("{SPEC}{body}"));
    assert!(report.ok && report.complete, "{report:#?}");

    let wrong = r#"
(defn unchanged {:t/sig '[{:tag keyword?} :-> {:tag keyword?}]} [input]
  (s/conform :server-capabilities/signature-help-provider input))
"#;
    let report = check(&format!("{SPEC}{wrong}"));
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert!(
        finding
            .actual
            .as_deref()
            .unwrap()
            .contains(":trigger-characters")
    );
}

#[test]
fn a_shadowed_cond_or_differing_result_fields_are_left_unchecked() {
    for source in [
        SPEC.replace(
            "(s/def :server-capabilities",
            "(defn cond [& _] nil)\n(s/def :server-capabilities",
        ),
        SPEC.replace(":else {:trigger-characters %}", ":else {:other %}"),
        SPEC.replace("(map? %) %", "(map? %) nil"),
    ] {
        let report = check(&source);
        assert!(!report.complete, "{report:#?}");
        assert!(report.diagnostics.iter().any(|finding| finding.code == "PNX9001"
            && finding.message.contains("s/conformer")), "{report:#?}");
    }
}

#[test]
fn feedback_with_the_same_map_shape_can_lose_literal_evidence() {
    let body = r#"
(defn until-stop
  {:t/sig '[(s/fspec :args (s/cat) :ret boolean?) {:tag #{:x}} :-> {:tag #{:x}}]}
  [again? input]
  (loop [value {:tag :x}]
    (if (again?)
      (recur input)
      (s/conform :server-capabilities/signature-help-provider value))))
(defn across-items
  {:t/sig '[{:tag #{:x}} (s/coll-of keyword? :kind vector?) :-> {:tag #{:x}}]}
  [input items]
  (reduce (fn [value _] (if (vector? value) :unexpected input)) {:tag :x} items))
"#;
    let report = check(&format!("{SPEC}{body}"));
    let [recur, reduce] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(recur.code, "PNX2322");
    assert!(
        recur
            .actual
            .as_deref()
            .unwrap()
            .contains(":trigger-characters")
    );
    assert_eq!(reduce.code, "PNX2322");
    assert!(reduce.actual.as_deref().unwrap().contains(":unexpected"));
}

use parenox::Session;

#[test]
fn numeric_selection_reports_an_unmodeled_stored_class() {
    let unknown = r#"
(ns demo.project)
(defn projected {:t/sig '[{1 string?} :-> any?]} [data]
  (select-keys data [1N]))
"#;
    let ordinary = r#"
(ns demo.project)
(defn projected {:t/sig '[:-> map?]} []
  (select-keys {1 "value"} [1N]))
"#;
    for source in [unknown, ordinary] {
        let report = Session::default().check("project.clj", source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert!(!report.complete);
        assert_eq!(finding.code, "PNX9001");
        assert!(finding.message.contains("selector's stored integer class"));
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            "[1N]"
        );
    }
}

const PROFILE: &str = include_str!("../examples/map_projection.clj");

#[test]
fn public_projection_checks_the_return_shape_and_rejects_an_internal_field() {
    let report = Session::default().check("profile.clj", PROFILE);
    assert!(report.ok && report.complete, "{report:#?}");

    for (keys, field) in [
        (
            r#"[:id :enabled "display" :internal-token]"#,
            ":internal-token",
        ),
        (r#"[:enabled "display"]"#, ":id"),
    ] {
        let edited = PROFILE.replace(r#"[:id :enabled "display"]"#, keys);
        let report = Session::default().check("profile.clj", &edited);
        assert!(!report.ok && report.complete, "{report:#?}");
        let [error] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(error.code, "PNX2322");
        let shape = if field == ":id" {
            &error.expected
        } else {
            &error.actual
        };
        assert!(shape.as_deref().unwrap().contains(field));
    }
}

#[test]
fn projection_keeps_nil_and_false_values_distinct_from_missing_keys() {
    let source = r#"
        (defn values {:t/sig '[:-> {:enabled #{false} :name nil?}]} []
          (select-keys {:enabled false :name nil :internal 42} '(:enabled :name :missing :name)))
        (defn omitted {:t/sig '[:-> #{:missing}]} []
          (get (values) :internal :missing))
        (defn empty-projections {:t/sig '[:-> (clojure.spec.alpha/tuple {} {} {})]} []
          [(select-keys nil [:unknown])
           (select-keys {:id 42} nil)
           (select-keys 42 [])])
    "#;
    let report = Session::default().check("presence.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn possible_selections_and_dictionary_entries_remain_optional() {
    let source = r#"
        (ns responses (:require [clojure.spec.alpha :as s]))
        (defn branch {:t/sig '[boolean? :-> (t/map {:req {:id number?}
                                                    :opt {:left string? :right boolean?}})]}
          [left?]
          (let [keys (if left? [:id :left] [:right :id])]
            (select-keys {:id 7 :left "left" :right true} keys)))
        (defn choice {:t/sig '[#{:id :left} :-> (t/map {:opt {:id number? :left string?}})]}
          [key]
          (select-keys {:id 7 :left "left" :right true} [key]))
        (defn dictionary {:t/sig '[(s/nilable (s/map-of keyword? string?))
                                   (s/coll-of #{:label} :kind vector?)
                                   :-> (t/map {:opt {:label string?}})]}
          [labels keys]
          (apply select-keys [labels keys]))
        (defn maybe-id {:t/sig '[(s/coll-of #{:id} :kind vector?)
                                 :-> (t/map {:opt {:id number?}})]}
          [keys]
          (select-keys {:id 7} keys))
    "#;
    let report = Session::default().check("optional.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    for (optional, required) in [
        (":opt {:label string?}", ":req {:label string?}"),
        (":opt {:id number?}", ":req {:id number?}"),
        (
            ":opt {:id number? :left string?}",
            ":req {:id number? :left string?}",
        ),
    ] {
        let edited = source.replace(optional, required);
        let report = Session::default().check("optional.clj", &edited);
        assert!(!report.ok && report.complete, "{report:#?}");
        let [error] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(error.code, "PNX2322");
    }
}

#[test]
fn projection_reports_invalid_selectors_and_unsupported_keys_at_the_argument() {
    let source = r#"
        (defn invalid {:t/sig '[:-> map?]} [] (select-keys {} false))
        (defn dynamic {:t/sig '[keyword? :-> map?]} [key] (select-keys {} [key]))
        (defn undeclared {:t/sig '[map? :-> map?]} [row] (select-keys row [:undeclared]))
    "#;
    let report = Session::default().check("diagnostics.clj", source);
    assert!(!report.ok && !report.complete, "{report:#?}");
    let [invalid, dynamic, undeclared] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    for (error, code, expression) in [
        (invalid, "PNX2322", "false"),
        (dynamic, "PNX9001", "[key]"),
        (undeclared, "PNX9003", "[:undeclared]"),
    ] {
        assert_eq!(error.code, code);
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            expression
        );
    }
    assert_eq!(
        invalid.contract.as_ref().unwrap().name,
        "clojure.core/select-keys"
    );
}

#[test]
fn a_present_abstract_child_does_not_inherit_its_defaults_literal_evidence() {
    let source = r#"
        (defn settled {:t/sig '[{1 #{:present}} :-> #{:present}]} [child]
          (let [{:keys [nested] :or {nested {1 :present}} :all result}
                {:nested child}]
            (get (:nested result) 1N :missing)))
    "#;
    let report = Session::default().check("unknown_child.clj", source);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete);
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.actual.as_deref(), Some(":missing | :present"));
}

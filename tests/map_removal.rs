use parenox::{Session, Source};

const PROFILE: &str = include_str!("../examples/map_projection.clj");

#[test]
fn a_removed_open_field_can_be_excluded_in_a_public_contract() {
    let exported = Session::default().export_externs(
        &[Source {
            path: "profile.clj",
            source: PROFILE,
        }],
        &[],
    );
    let edn = exported
        .edn
        .unwrap_or_else(|| panic!("{:#?}", exported.report));
    let client = r#"
        (ns client (:require [demo.map-projection :as profiles]))
        (defn has-token? {:t/sig '[:-> #{false}]} []
          (contains? (profiles/redact-profile {:id "id" :internal-token "private"})
                     :internal-token))
    "#;
    let report = Session::default().check_project_with_externs(
        &[Source {
            path: "client.clj",
            source: client,
        }],
        &[Source {
            path: "profile.edn",
            source: &edn,
        }],
    );
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = PROFILE.replace("(dissoc profile :internal-token)", "(dissoc profile)");
    let report = Session::default().check("profile.clj", &edited);
    assert!(!report.ok && report.complete, "{report:#?}");
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert!(error.actual.as_deref().unwrap().contains(":internal-token"));
}

#[test]
fn removal_masks_extra_entries_and_preserves_other_fields() {
    let source = r#"
        (ns removal (:require [clojure.spec.alpha :as s]))
        (defn absent {:t/sig '[map? :-> #{false}]} [row]
          (contains? (dissoc row "internal") "internal"))
        (defn fallback {:t/sig '[(s/map-of keyword? string?) :-> #{:fallback}]} [row]
          (get (dissoc row :secret) :secret :fallback))
        (defn projected {:t/sig '[map? :-> {}]} [row]
          (select-keys (dissoc row :secret) [:secret]))
        (defn dictionary {:t/sig '[(s/map-of #{:keep :remove} string?)
                                  :-> (s/map-of #{:keep} string?)]} [row]
          (dissoc row :remove))
        (defn replace {:t/sig '[map? :-> #{"replacement"}]} [row]
          (-> row (dissoc :secret) (assoc :secret "replacement") (get :secret)))
        (defn closed {:t/sig '[:-> {"id" nil? :enabled #{false}}]} []
          (apply dissoc [{:id 1 "id" nil :private "token" :enabled false} :id :private :absent]))
    "#;
    let report = Session::default().check("removal.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn a_dynamic_key_only_weakens_overlapping_required_fields() {
    let source = r#"
        (defn remove-key
          {:t/sig '[keyword? :-> (t/map {:req {"id" number?} :opt {:id string?}})]}
          [key]
          (dissoc {:id "internal" "id" 7} key))
        (defn remove-alternative
          {:t/sig '[#{:a :b} :-> (t/map {:req {:c number?} :opt {:a number? :b number?}})]}
          [key]
          (dissoc {:a 1 :b 2 :c 3} key))
    "#;
    let report = Session::default().check("dynamic.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace(
        r#":req {"id" number?} :opt {:id string?}"#,
        r#":req {"id" number? :id string?}"#,
    );
    let report = Session::default().check("dynamic.clj", &edited);
    assert!(!report.ok && report.complete, "{report:#?}");
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert!(error.actual.as_deref().unwrap().contains(":id?"));
}

#[test]
fn abstract_map_removal_does_not_assume_ordinary_numeric_key_equivalence() {
    let source = r#"
        (defn f {:t/sig '[{1 keyword?} :-> map?]} [value]
          (dissoc value 1N))
    "#;
    let report = Session::default().check("abstract.clj", source);
    let [gap] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete);
    assert_eq!(gap.code, "PNX9001");
    assert_eq!(&source[gap.span.start.offset..gap.span.end.offset], "1N");
}

#[test]
fn nil_and_identity_follow_jvm_arity_semantics() {
    let source = r#"
        (defn identity-result {:t/sig '[:-> number?]} [] (dissoc 42))
        (defn nil-result {:t/sig '[:-> nil?]} [] (dissoc nil :a 42))
        (defn nilable-map
          {:t/sig '[(clojure.spec.alpha/nilable {:removed number? :kept string?})
                    :-> (clojure.spec.alpha/nilable {:kept string?})]}
          [row]
          (dissoc row :removed))
    "#;
    let report = Session::default().check("arity.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    for (form, code) in [("(dissoc)", "PNX2554"), ("(dissoc 42 :id)", "PNX2322")] {
        let source = format!("(defn run {{:t/sig '[:-> nil?]}} [] {form})");
        let report = Session::default().check("arity.clj", &source);
        let [error] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert!(!report.ok && report.complete, "{report:#?}");
        assert_eq!(error.code, code);
        assert_eq!(error.contract.as_ref().unwrap().name, "clojure.core/dissoc");
    }
}

#[test]
fn an_open_apply_spread_cannot_prove_that_a_key_was_removed() {
    let source = r#"
        (ns removal (:require [clojure.spec.alpha :as s]))
        (defn removed {:t/sig '[(s/coll-of #{:id} :kind vector?) :-> {}]} [keys]
          (apply dissoc {:id 7} keys))
    "#;
    let report = Session::default().check("spread.clj", source);
    assert!(!report.ok && !report.complete, "{report:#?}");
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX9001");
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        "keys"
    );
}

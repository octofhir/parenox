use parenox::{Session, Source};

#[test]
fn runtime_set_predicates_exclude_falsey_and_invalid_members() {
    let provider = Source {
        path: "flags.clj",
        source: r#"(ns example.flags (:require [clojure.spec.alpha :as s]))
(s/def ::flags #{nil false true :yes ::s/invalid})
(s/def ::nothing #{nil false})"#,
    };
    let consumer = Source {
        path: "consumer.clj",
        source: r#"(ns example.consumer (:require [example.flags :as f] [clojure.spec.alpha :as s]))
(defn valid {:t/sig '[:-> (s/tuple #{false} #{false} #{true} #{true} #{false} #{false})]} []
  [(s/valid? ::f/flags nil) (s/valid? ::f/flags false) (s/valid? ::f/flags true)
   (s/valid? ::f/flags :yes) (s/valid? ::f/flags ::s/invalid) (s/valid? ::f/nothing nil)])
(defn conformed {:t/sig '[:-> (s/tuple #{::s/invalid} #{:yes})]} []
  [(s/conform ::f/flags false) (s/conform ::f/flags :yes)])
(defn declared {:t/sig '[::f/flags :-> #{true}]} [flag]
  (boolean flag))
(defn plain-type {:t/sig '[#{nil false} :-> #{false}]} [v]
  (boolean v))"#,
    };
    let mut session = Session::default();
    let report = session.check_project(&[consumer, provider], None);
    assert!(report.ok && report.complete, "{report:#?}");

    let changed = provider.source.replace("true :yes", "true :no");
    let edited = Source {
        source: &changed,
        ..provider
    };
    let warm = session.check_project(&[consumer, edited], None);
    let cold = Session::default().check_project(&[consumer, edited], None);
    assert!(!warm.ok && warm.complete, "{warm:#?}");
    assert_eq!(
        serde_json::to_value(&warm.files[0].diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
    let restored = session.check_project(&[consumer, provider], None);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

const ROW: &str = r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(s/def ::title string?)
(s/def ::row (s/keys :req-un [::title]))
"#;

#[test]
fn validates_a_database_row_and_checks_typed_uses_after_the_guard() {
    let source = format!(
        r#"{ROW}
(defn checked-row {{:t/sig '[map? :-> ::row]}} [row]
  (if (s/valid? ::row row) row (throw (ex-info "invalid row" {{}}))))
(defn display {{:t/sig '[map? :-> string?]}} [row]
  (let [tagged (assoc row :prefix "Title: ")]
    (when-not (not (s/valid? ::row tagged))
      (subs (:prefix tagged) 0))
    (if (s/valid? ::row tagged)
      (str (:prefix tagged) (subs (:title tagged) 0))
      "")))"#
    );
    let passed = Session::default().check("demo.clj", &source);
    assert!(passed.ok && passed.complete, "{passed:#?}");

    let bad = source.replace("(subs (:title tagged) 0)", "(inc (:title tagged))");
    let failed = Session::default().check("demo.clj", &bad);
    let [diagnostic] = failed.diagnostics.as_slice() else {
        panic!("{failed:#?}")
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.actual.as_deref(), Some("String"));
    assert_eq!(
        &bad[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "(:title tagged)"
    );
}

#[test]
fn a_successful_projection_does_not_prove_that_validation_returns_true() {
    let source = format!(
        r#"{ROW}
(s/def ::extra number?)
(defn bad-fallback {{:t/sig '[:-> number?]}} []
  (if (s/valid? ::row {{:title "valid shape" ::extra "bad extra key"}})
    5
    (inc "checked failure")))"#
    );
    let report = Session::default().check("demo.clj", &source);
    let [gap, mismatch] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(gap.code, "PNX9001");
    assert!(gap.message.contains("undeclared qualified key"));
    assert_eq!(mismatch.code, "PNX2322");
    assert_eq!(
        &source[mismatch.span.start.offset..mismatch.span.end.offset],
        "\"checked failure\""
    );
}

#[test]
fn resolves_referred_validation_and_aliases_without_refining_a_shadowed_call() {
    let app = r#"
(ns app (:require [clojure.spec.alpha :as s :refer [valid?] :rename {valid? checks?}]
                  [demo :as d]))
(defn checked-title {:t/sig '[map? :-> string?]} [row]
  (if (checks? ::d/row row) (subs (:title row) 0) "none"))
"#;
    let provider = Source {
        path: "demo.clj",
        source: ROW,
    };
    let report = Session::default().check_project(
        &[
            provider,
            Source {
                path: "app.clj",
                source: app,
            },
        ],
        None,
    );
    assert!(report.ok && report.complete, "{report:#?}");

    let shadowed = format!(
        r#"{app}
(defn shadowed
  {{:t/sig '[(s/fspec :args (s/cat :spec keyword? :value map?) :ret boolean?)
             map? :-> ::d/row]}}
  [checks? row]
  (if (checks? ::d/row row)
    row
    (throw (ex-info "invalid row" {{}}))))"#
    );
    let report = Session::default().check_project(
        &[
            provider,
            Source {
                path: "app.clj",
                source: &shadowed,
            },
        ],
        None,
    );
    let diagnostics: Vec<_> = report
        .files
        .iter()
        .flat_map(|file| &file.diagnostics)
        .collect();
    let [diagnostic] = diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert!(diagnostic.message.contains("shadowed"), "{report:#?}");
}

#[test]
fn unknown_validations_are_boolean_but_explicitly_incomplete() {
    let source = format!(
        r#"{ROW}
(s/def ::callback (s/fspec :args (s/cat :x string?) :ret string?))
(s/def ::callback-row (s/keys :req-un [::callback]))
(defn dynamic-check {{:t/sig '[keyword? map? :-> boolean?]}} [key row]
  (s/valid? key row))
(defn missing-check {{:t/sig '[map? :-> boolean?]}} [row]
  (s/valid? ::missing row))
(defn callback-check {{:t/sig '[map? :-> boolean?]}} [row]
  (s/valid? ::callback-row row))"#
    );
    let report = Session::default().check("demo.clj", &source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 3, "{report:#?}");
    for (diagnostic, expected) in
        report
            .diagnostics
            .iter()
            .zip(["literal", "No source declaration", "unchecked spec"])
    {
        assert_eq!(diagnostic.code, "PNX9001");
        assert_eq!(diagnostic.severity, "warning");
        assert!(diagnostic.message.contains(expected), "{report:#?}");
    }
}

#[test]
fn validation_checks_arity_but_does_not_run_after_a_throwing_argument() {
    let source = format!(
        r#"{ROW}
(defn short-call {{:t/sig '[:-> boolean?]}} [] (s/valid? ::row))
(defn long-call {{:t/sig '[:-> boolean?]}} [] (s/valid? ::row {{}} nil nil))
(defn argument-throws {{:t/sig '[:-> string?]}} []
  (try
    (s/valid? ::missing (throw (ex-info "stop" {{}})))
    (catch clojure.lang.ExceptionInfo _ "caught")))
(defn form-throws {{:t/sig '[:-> string?]}} []
  (try
    (s/valid? ::missing {{}} (throw (ex-info "stop in form" {{}})))
    (catch clojure.lang.ExceptionInfo _ "caught")))"#
    );
    let report = Session::default().check("demo.clj", &source);
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    for diagnostic in report.diagnostics {
        assert_eq!(diagnostic.code, "PNX2554");
        assert!(diagnostic.message.contains("2 or 3 arguments"));
    }
}

#[test]
fn the_optional_form_still_allows_refinement_by_a_literal_named_spec() {
    let source = format!(
        r#"{ROW}
(defn checked-title {{:t/sig '[map? :-> string?]}} [row]
  (if (s/valid? ::row row 'context)
    (subs (:title row) 0)
    ""))"#
    );
    let report = Session::default().check("demo.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");
}

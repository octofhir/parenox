use parenox_analysis::{Report, Session, Source};

fn specs() -> &'static str {
    include_str!("../../../tests/fixtures/spec-documentation.clj")
        .split_once(";; JVM-only")
        .unwrap()
        .0
}

fn check(body: &str) -> Report {
    Session::default().check("documentation.clj", &format!("{}{body}", specs()))
}

#[test]
fn documentation_conforms_both_branches_and_validates_exact_inputs() {
    let mut uses = r#"
(defn text {:t/sig '[:-> #{"docs"}]} [] (s/conform ::documentation "docs"))
(defn empty-text {:t/sig '[:-> #{""}]} [] (s/conform ::map-first ""))
(defn text-valid {:t/sig '[:-> #{true}]} [] (s/valid? ::map-first "docs"))
(defn markup {:t/sig '[:-> {:kind #{"markdown"} :value #{"content"}}]} []
  (s/conform ::map-first {:kind "markdown" :value "content"}))
(defn markup-valid {:t/sig '[:-> #{true}]} []
  (s/valid? ::documentation {:kind "plaintext" :value "content"}))
(defn guarded-map {:t/sig '[map? :-> string?]} [m]
  (if (s/valid? ::documentation m) (:value m) "fallback"))
(defn typed-choice
  {:t/sig '[(s/or :text #{"plain"} :row {:kind #{"markdown"} :value #{"body"}})
            :-> (s/or :text #{"plain"} :row {:kind #{"markdown"} :value #{"body"}})]}
  [input] (s/conform ::documentation input))
"#
    .to_owned();
    for (index, input) in [
        "nil",
        "false",
        "::s/invalid",
        ":markdown",
        "1",
        "[]",
        "{}",
        r#"{:kind "markdown"}"#,
        r#"{:kind "other" :value "x"}"#,
        r#"{:kind "plaintext" :value false}"#,
        r#"{:kind "markdown" :value "x" :extra/line nil}"#,
    ]
    .into_iter()
    .enumerate()
    {
        uses.push_str(&format!(
            r#"
(defn invalid-{index} {{:t/sig '[:-> #{{::s/invalid}}]}} [] (s/conform ::documentation {input}))
(defn invalid-{index}? {{:t/sig '[:-> #{{false}}]}} [] (s/valid? ::map-first {input}))
"#
        ));
    }
    let report = check(&uses);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn tagged_projection_preserves_or_replaces_map_entries_under_nested_conformance() {
    let uses = r#"
(def lhs {:documentation "docs"})
(def rhs {:documentation {:kind "markdown" :value "content"}})
(def changed {:documentation {:kind "markdown" :value "content" :extra/line 1N}})
(defn preserved-key {:t/sig '[:-> #{"docs"}]} []
  (:documentation (key (s/conform ::wrapped-docs (clojure.lang.MapEntry/create lhs rhs)))))
(defn preserved-val {:t/sig '[:-> #{"markdown"}]} []
  (:kind (:documentation (val (s/conform ::wrapped-docs (clojure.lang.MapEntry/create lhs rhs))))))
(defn converted-nested {:t/sig '[:-> int?]} []
  (let [conformed (s/conform ::wrapped-docs (clojure.lang.MapEntry/create lhs changed))]
    (:extra/line (:documentation (first (rest conformed))))))
(defn converted-map {:t/sig '[:-> int?]} []
  (:extra/line (s/conform ::documentation {:kind "markdown" :value "content" :extra/line 1N})))
(defn invalid-entry {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::wrapped-docs (clojure.lang.MapEntry/create lhs {})))
"#;
    let report = check(uses);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{uses}
(defn rebuilt {{:t/sig '[:-> any?]}} []
  (key (s/conform ::wrapped-docs (clojure.lang.MapEntry/create lhs changed))))
"#
    ));
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert!(finding.message.contains("key"), "{finding:#?}");
}

#[test]
fn documentation_propagates_map_conformer_errors_after_required_field_checks() {
    let report = check(
        r#"
(defn required-first {:t/sig '[:-> #{::s/invalid}]} []
  (try (s/conform ::documentation {:extra/line 2147483648})
       (catch ArithmeticException _ :unreachable)))
(defn conforms-overflow {:t/sig '[:-> #{:overflow}]} []
  (try (s/conform ::documentation {:kind "markdown" :value "x" :extra/line 2147483648})
       (catch ArithmeticException _ :overflow)))
(defn validates-overflow {:t/sig '[:-> #{:overflow}]} []
  (try (s/valid? ::documentation {:kind "markdown" :value "x" :extra/line 2147483648})
       (catch ArithmeticException _ :overflow)))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn the_documentation_input_union_imports_as_an_extern_without_sources() {
    let provider = format!(
        r#"{}
(defn emit {{:t/sig '[::documentation :-> keyword?]}} [_] :emitted)
"#,
        specs()
    );
    let provider = Source {
        path: "provider.clj",
        source: &provider,
    };
    let mut session = Session::default();
    let exported = session.export_externs(&[provider], &[]);
    assert!(
        exported.report.ok && exported.report.complete,
        "{exported:#?}"
    );
    let edn = exported.edn.unwrap();
    let externs = Source {
        path: "provider.edn",
        source: &edn,
    };
    let validation = session.validate_externs(&[provider], &[externs]);
    assert!(validation.ok && validation.complete, "{validation:#?}");
    let check = |body: &str| {
        let source = format!(
            "(ns consumer (:require [clojure.spec.alpha :as s] [demo.documentation :as docs]))\n{body}"
        );
        Session::default().check_project_with_externs(
            &[Source {
                path: "consumer.clj",
                source: &source,
            }],
            &[externs],
        )
    };
    let accepted = check(
        r#"(defn send {:t/sig '[string? :-> keyword?]} [x]
             (docs/emit x)
             (docs/emit {:kind "plaintext" :value x}))"#,
    );
    assert!(accepted.ok && accepted.complete, "{accepted:#?}");

    for input in [
        "::s/invalid",
        r#"{:kind "other" :value "x"}"#,
        r#"{:kind "markdown" :value 1}"#,
        r#"{:kind "plaintext"}"#,
    ] {
        let body = format!(r#"(defn sends {{:t/sig '[:-> keyword?]}} [] (docs/emit {input}))"#);
        let report = check(&body);
        let [finding] = report.files[0].diagnostics.as_slice() else {
            panic!("{input}: {report:#?}");
        };
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().path.as_deref(),
            Some("provider.edn")
        );
    }
}

#[test]
fn string_branch_still_requires_the_documentation_registry_owner() {
    let provider = Source {
        path: "demo/documentation.clj",
        source: specs(),
    };
    for (libspec, loaded) in [(":as-alias", false), (":as", true)] {
        let source = format!(
            r#"
(ns consumer (:require [clojure.spec.alpha :as s] [demo.documentation {libspec} docs]))
(defn text {{:t/sig '[:-> #{{"text"}}]}} [] (s/conform ::docs/documentation "text"))
(defn valid {{:t/sig '[:-> #{{true}}]}} [] (s/valid? ::docs/documentation "text"))
"#
        );
        let report = Session::default().check_project(
            &[
                Source {
                    path: "consumer.clj",
                    source: &source,
                },
                provider,
            ],
            None,
        );
        if loaded {
            assert!(report.ok && report.complete, "{report:#?}");
        } else {
            let findings = &report.files[0].diagnostics;
            assert_eq!(findings.len(), 2, "{report:#?}");
            assert!(findings.iter().all(|finding| {
                finding.code == "PNX9001" && finding.message.contains("registrations are loaded")
            }));
        }
    }
}

#[test]
fn documentation_requires_named_map_registration_before_any_eager_resolution() {
    let target = r#"
(s/def ::kind #{"markdown"})
(s/def ::markup (s/keys :req-un [::kind]))
"#;
    let alias = "(s/def ::markup-alias ::markup)";
    let early_call = r#"
(s/def ::probe
  (s/with-gen string?
    (try (s/conform ::documentation "early")
      (catch Exception _ (fn [] nil)))))
"#;
    for (branch, registrations, checked) in [
        ("::markup-alias", format!("{alias}{target}"), false),
        ("::markup", format!("{early_call}{target}"), false),
        ("::markup-alias", format!("{target}{alias}"), true),
    ] {
        let source = format!(
            r#"
(ns demo.documentation-order (:require [clojure.spec.alpha :as s]))
(s/def ::documentation
  (s/and (s/or :string string? :markup {branch}) (s/conformer second)))
{registrations}
(defn value {{:t/sig '[:-> #{{"later"}}]}} [] (s/conform ::documentation "later"))
"#
        );
        let report = Session::default().check("documentation_order.clj", &source);
        if checked {
            assert!(report.ok && report.complete, "{source}\n{report:#?}");
        } else {
            let later_call = source.rfind("::documentation").unwrap();
            assert!(!report.complete, "{source}\n{report:#?}");
            assert!(
                report.diagnostics.iter().any(|finding| {
                    finding.code == "PNX9001"
                        && finding.message.contains("supported conversion")
                        && finding.span.start.offset == later_call
                }),
                "{source}\n{report:#?}"
            );
        }
    }
}

#[test]
fn tagged_projections_require_local_map_conversions_and_core_second() {
    let local = r#"
(ns demo.choices
  (:refer-clojure :exclude [second])
  (:require [clojure.spec.alpha :as s] [demo.foreign :as foreign]))
(defn second {:t/sig '[any? :-> #{:different}]} [_] :different)
(s/def ::kind #{"markdown"})
(s/def ::value string?)
(s/def ::local (s/keys :req-un [::kind ::value]))
(s/def ::contains-foreign-fields (s/keys :req-un [:foreign/kind ::value]))
(s/def ::canonical
  (s/and (s/or :text string? :markup ::local) (s/conformer clojure.core/second)))
(s/def ::shadowed
  (s/and (s/or :text string? :markup ::local) (s/conformer second)))
(s/def ::inline
  (s/and (s/or :text string? :markup (s/keys :req-un [::kind ::value]))
         (s/conformer clojure.core/second)))
(s/def ::foreign-map
  (s/and (s/or :text string? :markup ::foreign/markup) (s/conformer clojure.core/second)))
(s/def ::foreign-field
  (s/and (s/or :text string? :markup ::contains-foreign-fields) (s/conformer clojure.core/second)))
(s/def ::nested
  (s/and (s/or :text string? :nested ::canonical) (s/conformer clojure.core/second)))
(defn canonical {:t/sig '[:-> #{"text"}]} [] (s/conform ::canonical "text"))
(defn shadowed {:t/sig '[:-> any?]} [] (s/conform ::shadowed "text"))
(defn inline {:t/sig '[:-> #{"text"}]} [] (s/conform ::inline "text"))
(defn foreign-map {:t/sig '[:-> any?]} [] (s/conform ::foreign-map "text"))
(defn foreign-field {:t/sig '[:-> any?]} [] (s/conform ::foreign-field "text"))
(defn nested {:t/sig '[:-> any?]} [] (s/conform ::nested "text"))
"#;
    let report = Session::default().check_project(
        &[
            Source {
                path: "demo/choices.clj",
                source: local,
            },
            Source {
                path: "demo/foreign.clj",
                source: r#"
(ns demo.foreign (:require [clojure.spec.alpha :as s]))
(s/def :foreign/kind #{"markdown"})
(s/def :foreign/value string?)
(s/def ::markup (s/keys :req-un [:foreign/kind :foreign/value]))
"#,
            },
        ],
        None,
    );
    let targeted: Vec<_> = report.files[0]
        .diagnostics
        .iter()
        .filter(|finding| {
            finding.code == "PNX9001" && finding.message.contains("supported conversion")
        })
        .map(|finding| &local[finding.span.start.offset..finding.span.end.offset])
        .collect();
    assert_eq!(
        targeted,
        ["::shadowed", "::foreign-map", "::foreign-field", "::nested"],
        "{report:#?}"
    );
}

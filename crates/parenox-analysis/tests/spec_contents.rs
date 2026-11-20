use parenox_analysis::{Report, Session, Source};

const MARKED: &str = r#"
(ns demo.contents (:require [clojure.spec.alpha :as s]))
(s/def ::marked-string
  (s/and (s/or :string string? :marked-string (s/keys :req-un [::language ::value]))
         (s/conformer second)))
(s/def :extra/line (s/and integer? (s/conformer int)))
"#;

const MARKUP: &str = r#"
(s/def :markup/kind #{"plaintext" "markdown"})
(s/def :markup/value string?)
(s/def ::markup-content (s/keys :req-un [:markup/kind :markup/value]))
"#;

const CONTENTS: &str = r#"
(s/def ::contents
  (s/and (s/or :marked-strings (s/coll-of ::marked-string) :markup-content ::markup-content)
         (s/conformer second)))
"#;

const REGISTERED: &str = r#"
(s/def ::language string?)
(s/def ::value string?)
"#;

fn check(body: &str) -> Report {
    Session::default().check("contents.clj", &format!("{MARKED}{MARKUP}{CONTENTS}{body}"))
}

fn assert_presence_warnings(report: &Report) {
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    for field in ["language", "value"] {
        assert!(
            report.diagnostics.iter().any(|finding| {
                finding.code == "PNX9001"
                    && finding.message.contains(&format!(":demo.contents/{field}"))
                    && finding.message.contains("s/keys checks presence only")
            }),
            "{report:#?}"
        );
    }
}

#[test]
fn marked_contents_preserve_ordered_collections_and_unchanged_entries() {
    let report = check(
        r#"
(def marked {:language false :value ::s/invalid})
(defn mixed-vector {:t/sig '[:-> (s/tuple #{"source"} {:language #{false} :value #{::s/invalid}})]} []
  (s/conform ::contents ["source" marked]))
(defn mixed-list {:t/sig '[:-> (s/tuple #{"source"} #{::s/invalid})]} []
  (let [result (s/conform ::contents (list "source" marked))]
    [(first result) (:value (first (rest result)))]))
(defn seq-builds-a-list
  {:t/sig '[:-> (s/coll-of (s/or :source string? :marked map?) :kind list?)]} []
  (s/conform ::contents (seq ["source" marked])))
(defn mixed-entry {:t/sig '[:-> (s/tuple #{"source"} #{false})]} []
  (let [result (s/conform ::contents (clojure.lang.MapEntry/create "source" marked))]
    [(key result) (:language (val result))]))
(defn collection-valid {:t/sig '[:-> #{true}]} []
  (s/valid? ::contents ["source" marked]))
"#,
    );
    assert_presence_warnings(&report);
}

#[test]
fn empty_maps_match_the_collection_and_nonempty_markup_falls_through() {
    let mut body = r#"
(defn empty-map {:t/sig '[:-> {}]} [] (s/conform ::contents {}))
(defn empty-valid {:t/sig '[:-> #{true}]} [] (s/valid? ::contents {}))
(defn markup {:t/sig '[:-> {:kind #{"markdown"} :value #{"body"}}]} []
  (s/conform ::contents {:kind "markdown" :value "body"}))
(defn markup-valid {:t/sig '[:-> #{true}]} []
  (s/valid? ::contents {:kind "plaintext" :value "body"}))
"#
    .to_owned();
    for (index, input) in [
        r#""raw string""#,
        "nil",
        "::s/invalid",
        "1",
        r#"{:language false :value nil}"#,
        r#"{:kind "other" :value "body"}"#,
        r#"[{:kind "markdown" :value "body"}]"#,
        r#"["text" nil]"#,
        r#"["text" {:language nil}]"#,
    ]
    .into_iter()
    .enumerate()
    {
        body.push_str(&format!(
            r#"
(defn invalid-{index} {{:t/sig '[:-> #{{::s/invalid}}]}} [] (s/conform ::contents {input}))
(defn invalid-{index}? {{:t/sig '[:-> #{{false}}]}} [] (s/valid? ::contents {input}))
"#
        ));
    }
    assert_presence_warnings(&check(&body));
}

#[test]
fn collection_and_markup_branches_apply_registered_extra_conversions_in_order() {
    let body = r#"
(def marked {:language "clojure" :value "form" :extra/line 1N})
(def overflow {:language "clojure" :value "form" :extra/line 2147483648})
(defn vector-extra {:t/sig '[:-> int?]} []
  (:extra/line (first (rest (s/conform ::contents ["text" marked])))))
(defn entry-extra {:t/sig '[:-> int?]} []
  (:extra/line (first (rest (s/conform ::contents (clojure.lang.MapEntry/create "text" marked))))))
(defn markup-extra {:t/sig '[:-> int?]} []
  (:extra/line (s/conform ::contents {:kind "markdown" :value "body" :extra/line 1N})))
(defn early-rejection {:t/sig '[:-> #{::s/invalid}]} []
  (try (s/conform ::contents [nil overflow])
       (catch ArithmeticException _ :unreachable)))
(defn early-overflow {:t/sig '[:-> #{:overflow}]} []
  (try (s/conform ::contents (list overflow nil))
       (catch ArithmeticException _ :overflow)))
(defn validation-overflow {:t/sig '[:-> #{:overflow}]} []
  (try (s/valid? ::contents [overflow nil])
       (catch ArithmeticException _ :overflow)))
"#;
    assert_presence_warnings(&check(body));

    let report = check(&format!(
        r#"{body}
(defn rebuilt {{:t/sig '[:-> any?]}} [] (key (s/conform ::contents (clojure.lang.MapEntry/create "text" marked))))
"#
    ));
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.severity == "error")
        .collect();
    let [finding] = errors.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert!(finding.message.contains("key"), "{finding:#?}");
    assert_eq!(report.diagnostics.len(), 3, "{report:#?}");
}

#[test]
fn literal_sets_conform_mixed_members_without_assuming_their_order() {
    let report = check(
        r#"
(def wide {:language "clojure" :value "form" :extra/line 2147483648})
(defn mixed-set
  {:t/sig '[:-> (s/coll-of (s/or :source #{"source"}
                               :marked {:language #{"clojure"} :value #{"form"} :extra/line int?})
                          :kind set?)]} []
  (s/conform ::contents #{"source" {:language "clojure" :value "form" :extra/line 1N}}))
(defn invalid-member {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::contents #{"source" {:kind "markdown" :value "not a marked member"}}))
(defn overflow {:t/sig '[:-> #{:overflow}]} []
  (try (s/conform ::contents #{"source" wide})
       (catch ArithmeticException _ :overflow)))
(defn unordered {:t/sig '[:-> #{::s/invalid :overflow}]} []
  (try (s/conform ::contents #{nil wide})
       (catch ArithmeticException _ :overflow)))
"#,
    );
    assert_presence_warnings(&report);
}

#[test]
fn nested_contents_preserve_entries_until_a_list_is_rebuilt() {
    let body = format!(
        r#"{REGISTERED}
(s/def ::row (s/keys :req-un [::contents]))
(s/def ::rows (s/coll-of ::row))
(defn unchanged {{:t/sig '[:-> (s/tuple #{{"source"}} #{{"markdown"}})]}} []
  (let [left {{:contents ["source" {{:language "clojure" :value "body"}}]}}
        right {{:contents {{:kind "markdown" :value "body"}}}}
        entry (s/conform ::rows (clojure.lang.MapEntry/create left right))]
    [(first (:contents (key entry))) (:kind (:contents (val entry)))]))
(defn unchanged-empty {{:t/sig '[:-> {{}}]}} []
  (:contents (key (s/conform ::rows (clojure.lang.MapEntry/create {{:contents {{}}}} {{:contents []}})))))
(defn rebuilt-list {{:t/sig '[:-> #{{"source"}}]}} []
  (let [left {{:contents (list "source")}}
        right {{:contents []}}
        result (s/conform ::rows (clojure.lang.MapEntry/create left right))]
    (first (:contents (first result)))))
"#
    );
    let report = check(&body);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{body}
(defn no-entry-key {{:t/sig '[:-> any?]}} []
  (let [left {{:contents (list "source")}}
        right {{:contents []}}]
    (key (s/conform ::rows (clojure.lang.MapEntry/create left right)))))
"#
    ));
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert!(finding.message.contains("key"), "{finding:#?}");
}

#[test]
fn checking_a_broad_map_does_not_prove_the_markup_keys_exist() {
    let report = check(&format!(
        r#"
{REGISTERED}
(defn preserve-map {{:t/sig '[map? :-> map?]}} [m]
  (if (s/valid? ::contents m) m {{}}))
(defn mandatory-kind {{:t/sig '[map? :-> string?]}} [m]
  (if (s/valid? ::contents m) (:kind m) "fallback"))
"#
    ));
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.severity, "error", "{finding:#?}");
    assert!(
        ["PNX2322", "PNX9004"].contains(&finding.code.as_str()),
        "{finding:#?}"
    );
    assert!(finding.message.contains("mandatory-kind"), "{finding:#?}");
}

#[test]
fn registered_contents_contracts_import_without_source() {
    let provider_text = format!(
        r#"
{MARKED}{REGISTERED}{MARKUP}{CONTENTS}
(defn accepts {{:t/sig '[::contents :-> keyword?]}} [_] :accepted)
"#
    );
    let provider = Source {
        path: "provider.clj",
        source: &provider_text,
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
    let validated = session.validate_externs(&[provider], &[externs]);
    assert!(validated.ok && validated.complete, "{validated:#?}");

    let check = |input| {
        let source = format!(
            r#"
(ns consumer (:require [demo.contents :as wire]))
(defn forwards {{:t/sig '[:-> keyword?]}} [] (wire/accepts {input}))
"#
        );
        Session::default().check_project_with_externs(
            &[Source {
                path: "consumer.clj",
                source: &source,
            }],
            &[externs],
        )
    };
    for accepted in [
        "{}",
        "[]",
        r#"["source" {:language "clojure" :value "body"}]"#,
        r#"["source"]"#,
        r#"{:kind "markdown" :value "body"}"#,
    ] {
        let report = check(accepted);
        assert!(report.ok && report.complete, "{accepted}: {report:#?}");
    }

    for rejected in [
        r#""source""#,
        r#"{:language "clojure" :value "body"}"#,
        r#"["source" {:value "body"}]"#,
        r#"["source" {:language nil :value "body"}]"#,
        r#"{:kind "other" :value "body"}"#,
    ] {
        let report = check(rejected);
        let [finding] = report.files[0].diagnostics.as_slice() else {
            panic!("{rejected}: {report:#?}");
        };
        assert_eq!(finding.code, "PNX2322", "{rejected}: {finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().path.as_deref(),
            Some("provider.edn")
        );
    }
}

#[test]
fn even_collection_inputs_require_registry_resolution_for_both_branches() {
    let unresolved = format!(
        r#"
{MARKED}{REGISTERED}{CONTENTS}
(defn later {{:t/sig '[:-> any?]}} [] (s/conform ::contents ["source"]))
"#
    );
    let eager = format!(
        r#"
{MARKED}{REGISTERED}{CONTENTS}
(s/def ::probe
  (s/with-gen string?
    (try (s/conform ::contents ["early"])
      (catch Exception _ (fn [] nil)))))
{MARKUP}
(defn later {{:t/sig '[:-> any?]}} [] (s/conform ::contents ["source"]))
"#
    );
    for source in [unresolved, eager] {
        let later_call = source.rfind("::contents").unwrap();
        let report = Session::default().check("contents.clj", &source);
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

    let lazy = format!(
        r#"
{MARKED}{REGISTERED}{CONTENTS}{MARKUP}
(defn later {{:t/sig '[:-> (s/tuple #{{"source"}})]}} [] (s/conform ::contents ["source"]))
"#
    );
    let report = Session::default().check("contents.clj", &lazy);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn alias_only_does_not_load_the_collection_projection() {
    let provider = format!("{MARKED}{REGISTERED}{MARKUP}{CONTENTS}");
    let provider = Source {
        path: "demo/contents.clj",
        source: &provider,
    };
    for (mode, loaded) in [(":as-alias", false), (":as", true)] {
        let source = format!(
            r#"
(ns consumer (:require [clojure.spec.alpha :as s] [demo.contents {mode} wire]))
(defn conformed {{:t/sig '[:-> (s/tuple #{{"source"}})]}} []
  (s/conform ::wire/contents ["source"]))
(defn valid {{:t/sig '[:-> #{{true}}]}} []
  (s/valid? ::wire/contents ["source"]))
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

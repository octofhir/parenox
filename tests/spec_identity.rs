use parenox::{Report, Session, Source};

const SPEC: &str = include_str!("fixtures/spec-identity.clj");

fn check(body: &str) -> Report {
    let source = format!("{}\n{body}", SPEC.split_once(";; JVM-only").unwrap().0);
    Session::default().check("identity_spec.clj", &source)
}

#[test]
fn checks_the_identity_arm_of_an_lsp_style_tagged_spec() {
    let caller = r#"
(s/fdef accept :args (s/cat :value ::any-or-error) :ret keyword?)
(defn accept [value] :accepted)
(defn send {:t/sig '[:-> keyword?]} []
  (accept {:message "error"})
  (accept "")
  (accept [])
  (accept 0))
"#;
    let report = check(caller);
    assert!(report.ok && report.complete, "{report:#?}");
    for invalid in ["nil", "false", "::s/invalid"] {
        let report = check(&caller.replace("(accept 0)", &format!("(accept {invalid})")));
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{invalid}: {report:#?}")
        };
        assert_eq!(finding.code, "PNX2322", "{invalid}: {finding:#?}");
    }

    let report = check(
        "(defn accept {:t/sig '[::truth :-> keyword?]} [x] :ok)\n
         (defn forward {:t/sig '[keyword? :-> keyword?]} [x] (accept x))",
    );
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9004", "{report:#?}");
}

#[test]
fn validates_named_identity_and_refines_truthy_branches() {
    let source = r#"
(s/def ::alias ::truth)
(s/def ::true (s/and boolean? identity))
(defn preserved {:t/sig '[:-> #{""}]} [] (s/conform ::alias ""))
(defn invalid {:t/sig '[:-> #{:clojure.spec.alpha/invalid}]} []
  (s/conform ::alias ::s/invalid))
(defn false-value {:t/sig '[:-> #{false}]} [] (s/valid? ::alias nil))
(defn conditional {:t/sig '[boolean? :-> ::true]} [x]
  (if (s/valid? ::alias x) x true))
(defn known-truthy {:t/sig '[::truth :-> ::true]} [x]
  (if (boolean? x) x true))
(defn ordinary-call {:t/sig '[:-> #{:clojure.spec.alpha/invalid}]} []
  (identity ::s/invalid))
"#;
    let report = check(source);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(
        r#"(defn keyword-branch {:t/sig '[::truth :-> number?]} [x]
             (if (keyword? x) "wrong" 1))"#,
    );
    assert_eq!(report.diagnostics[0].code, "PNX2322", "{report:#?}");
}

#[test]
fn a_shadowed_predicate_stays_unchecked_and_qualified_core_still_works() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
(defn identity {:t/sig '[string? :-> string?]} [x] x)
(s/def ::local identity)
(s/def ::qualified clojure.core/identity)
(defn qualified {:t/sig '[:-> #{true}]} [] (s/valid? ::qualified 0))
"#;
    let report = Session::default().check("demo.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete);
    assert!(
        finding
            .message
            .contains("Source predicate identity is unchecked")
    );
}

#[test]
fn exports_and_reads_a_truthy_argument_contract() {
    let source = Source {
        path: "provider.clj",
        source: "(ns provider) (defn accept {:t/sig '[identity :-> string?]} [x] \"ok\")",
    };
    let export = Session::default().export_externs(&[source], &[]);
    let edn = export.edn.unwrap_or_else(|| panic!("{:#?}", export.report));
    let externs = [Source {
        path: "provider.edn",
        source: &edn,
    }];
    let client = r#"(ns client (:require [provider :as p]))
(defn call {:t/sig '[:-> string?]} [] (p/accept 0))"#;
    let check = |source: &str| {
        Session::default().check_project_with_externs(
            &[Source {
                path: "client.clj",
                source,
            }],
            &externs,
        )
    };
    let report = check(client);
    assert!(report.ok && report.complete, "{report:#?}");
    let report = check(&client.replace("(p/accept 0)", "(p/accept false)"));
    assert_eq!(
        report.files[0].diagnostics[0].code, "PNX2322",
        "{report:#?}"
    );
}

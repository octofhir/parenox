use parenox::{ContractKind, Session, Source};

const PROVIDER: Source<'_> = Source {
    path: "labels.clj",
    source: r#"(ns labels (:require [clojure.spec.alpha :as spec]))
        (spec/def ::label string?)
        (spec/fdef render
          :args (spec/alt :with-n (spec/cat :n number? :label ::label)
                          :empty (spec/cat)
                          :one (spec/cat :label ::label))
          :ret string?)
        (defn render
          ([] "empty")
          ([label] (clojure.string/upper-case label))
          ([n label] (str (inc n) label)))
        (defn from-here {:t/sig '[:-> string?]} []
          (str (render) (render "a") (render 1 "b")))"#,
};

const CONSUMER: Source<'_> = Source {
    path: "client.clj",
    source: r#"(ns client (:require [labels :as api]))
        (defn render-all {:t/sig '[:-> string?]} []
          (str (api/render) (api/render "a") (api/render 1 "b")))"#,
};

#[test]
fn each_arity_checks_its_body_and_calls_across_namespaces() {
    let mut session = Session::default();
    let accepted = session.check_project(&[CONSUMER, PROVIDER], None);
    assert!(accepted.ok && accepted.complete, "{accepted:#?}");

    let provider = PROVIDER.source.replace("([] \"empty\")", "([] false)");
    let provider = Source {
        source: &provider,
        ..PROVIDER
    };
    let consumer = CONSUMER
        .source
        .replace("api/render \"a\"", "api/render :bad-one")
        .replace("api/render 1 \"b\"", "api/render :bad-two \"b\"");
    let consumer = Source {
        source: &consumer,
        ..CONSUMER
    };
    let rejected = session.check_project(&[consumer, provider], None);
    assert!(!rejected.ok && rejected.complete, "{rejected:#?}");
    for (file, source, expected) in [
        (
            &rejected.files[0],
            consumer.source,
            &[":bad-one", ":bad-two"][..],
        ),
        (&rejected.files[1], provider.source, &["false"][..]),
    ] {
        assert_eq!(file.diagnostics.len(), expected.len(), "{file:#?}");
        for (diagnostic, text) in file.diagnostics.iter().zip(expected) {
            assert_eq!(diagnostic.code, "PNX2322", "{diagnostic:#?}");
            let origin = diagnostic.contract.as_ref().unwrap();
            assert_eq!(origin.kind, ContractKind::SFdef);
            assert_eq!(origin.name, "labels/render");
            assert_eq!(origin.path.as_deref(), Some("labels.clj"));
            assert_eq!(
                &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
                *text
            );
        }
    }
}

#[test]
fn supported_fdef_alternatives_must_match_the_defn_and_inline_signature() {
    let source = r#"(ns example (:require [clojure.spec.alpha :as s :refer [cat alt]]))
      (s/fdef label :ret string?
        :args (alt :two (cat :n number? :text string?) :one (cat :text string?)))
      (defn label {:t/sig '[[string? :-> string?] [number? string? :-> string?]]}
        ([text] text) ([_ text] text))"#;
    let mut session = Session::default();
    let correct = session.check("example.clj", source);
    assert!(correct.ok && correct.complete, "{correct:#?}");

    let wrong = source.replace("[number? string? :->", "[boolean? string? :->");
    let rejected = session.check("example.clj", &wrong);
    assert_eq!(rejected.diagnostics.len(), 1, "{rejected:#?}");
    assert_eq!(rejected.diagnostics[0].code, "PNX2790");

    let wrong = source
        .replace("(alt :two (cat :n number? :text string?) :one", "(alt :one")
        .replace(
            "{:t/sig '[[string? :-> string?] [number? string? :-> string?]]}",
            "",
        );
    let rejected = session.check("example.clj", &wrong);
    assert_eq!(rejected.diagnostics.len(), 1, "{rejected:#?}");
    assert_eq!(rejected.diagnostics[0].code, "PNX2554");
}

#[test]
fn overlapping_and_non_fixed_regexes_leave_the_contract_unchecked() {
    for (branch, message) in [
        ("(s/cat :text string?)", "overlapping arities"),
        ("(s/cat :xs (s/* string?))", "nested argument regexes"),
        ("(s/cat :xs (s/? string?))", "nested argument regexes"),
        (
            "(s/cat :xs (s/cat :text string?))",
            "nested argument regexes",
        ),
        ("(s/+ string?)", "requires fixed s/cat branches"),
    ] {
        let source = format!(
            r#"(ns example (:require [clojure.spec.alpha :as s]))
            (s/fdef f :args (s/alt :first (s/cat :x number?) :second {branch}) :ret number?)
            (defn f [x] x)"#
        );
        let report = Session::default().check("example.clj", &source);
        assert!(!report.complete, "{report:#?}");
        assert_eq!(report.stats.checked_functions, 0, "{report:#?}");
        let [gap] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(gap.code, "PNX9001");
        assert_eq!(gap.severity, "warning");
        assert!(gap.message.contains(message), "{gap:#?}");
    }
}

#[test]
fn an_unchecked_argument_spec_cannot_establish_fixed_arity() {
    let source = r#"(ns example (:require [clojure.spec.alpha :as s]))
        (s/def ::maybe (s/? string?))
        (s/fdef f :args (s/alt :empty (s/cat) :one (s/cat :x ::maybe)) :ret string?)
        (defn f ([] "empty") ([x] x))"#;
    let report = Session::default().check("example.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 0, "{report:#?}");
    assert!(report.diagnostics.iter().any(|gap| {
        gap.code == "PNX9001" && gap.message.contains("Cannot determine the arity")
    }));
}

#[test]
fn fspec_does_not_gain_fdef_regex_overloads() {
    let source = r#"(ns example (:require [clojure.spec.alpha :as s]))
        (defn f {:t/sig '[(s/fspec :args (s/alt :zero (s/cat) :one (s/cat :x number?))
                               :ret number?) :-> nil?]} [_] nil)"#;
    let report = Session::default().check("example.clj", source);
    assert!(!report.complete, "{report:#?}");
    let [gap] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(gap.code, "PNX9001");
    assert!(gap.message.contains("s/fspec :args requires"));
}

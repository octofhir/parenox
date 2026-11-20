use parenox::{ProjectReport, Session, Source};

fn check(consumer: &str) -> ProjectReport {
    let (wire, _) = include_str!("fixtures/spec-int-conformer.clj")
        .split_once(";; JVM-only assertions:")
        .unwrap();
    Session::default().check_project(
        &[
            Source {
                path: "src/lsp4clj/coercer.clj",
                source: wire,
            },
            Source {
                path: "src/adapter.clj",
                source: consumer,
            },
        ],
        None,
    )
}

const NS: &str =
    "(ns adapter (:require [clojure.spec.alpha :as s] [lsp4clj.coercer :as-alias wire]))";

#[test]
fn checks_the_lsp_error_code_spec_at_typed_calls() {
    let source = format!(
        r#"{NS}
(s/fdef accept :args (s/cat :code :error/code) :ret keyword?)
(defn accept [code] :accepted)
(defn forward {{:t/sig '[:-> keyword?]}} []
  (accept 2147483648)
  (accept :failed)
  (accept 2/2)
  (accept 36r1N)
  (accept (s/conform ::wire/line 1N)))
"#
    );
    let report = check(&source);
    assert!(report.ok && report.complete, "{report:#?}");

    for input in [
        "1N",
        "\"failed\"",
        "1/2",
        "9223372036854775808/9223372036854775808",
    ] {
        let bad = source.replace("2147483648", input);
        let report = check(&bad);
        let [finding] = report.files[1].diagnostics.as_slice() else {
            panic!("{input}: {report:#?}")
        };
        assert_eq!(
            (finding.code.as_str(), finding.severity),
            ("PNX2322", "error")
        );
        assert_eq!(
            &bad[finding.span.start.offset..finding.span.end.offset],
            input
        );
    }
}

#[test]
fn class_unknown_is_unproven_even_with_a_proven_range() {
    let source = format!(
        r#"{NS}
(defn accept {{:t/sig '[:error/code :-> keyword?]}} [x] :accepted)
(defn from-integer {{:t/sig '[integer? :-> keyword?]}} [x] (accept x))
(defn from-int-range {{:t/sig '[::wire/line :-> keyword?]}} [x] (accept x))
"#
    );
    let report = check(&source);
    assert!(!report.complete, "{report:#?}");
    let findings = &report.files[1].diagnostics;
    assert_eq!(findings.len(), 2, "{report:#?}");
    assert!(
        findings
            .iter()
            .all(|finding| finding.code == "PNX9004" && finding.severity == "error"),
        "{report:#?}"
    );
}

#[test]
fn int_predicate_narrows_conditions_without_losing_a_known_range() {
    let imports = NS.replace(":as-alias", ":as");
    let source = format!(
        r#"{imports}
(defn guarded {{:t/sig '[integer? :-> :error/code]}} [x]
  (if (int? x) x :failed))
(defn guarded-in-range {{:t/sig '[::wire/line :-> ::wire/line]}} [x]
  (if (int? x) x 0))
(defn checked-spec {{:t/sig '[number? :-> :error/code]}} [x]
  (if (s/valid? :error/code x) x :failed))
(defn wide {{:t/sig '[:-> #{{true}}]}} [] (int? 2147483648))
(defn arbitrary {{:t/sig '[:-> #{{false}}]}} [] (int? 1N))
(defn general {{:t/sig '[integer? :-> boolean?]}} [x] (int? x))
(defn converted {{:t/sig '[:-> #{{true}}]}} [] (int? (s/conform ::wire/line 1N)))
"#
    );
    let report = check(&source);
    assert!(report.ok && report.complete, "{report:#?}");

    for (old, new, function) in [
        (
            "[integer? :-> boolean?]",
            "[integer? :-> #{false}]",
            "general",
        ),
        (
            "(if (int? x) x 0)",
            "(if (int? x) \"bad\" 0)",
            "guarded-in-range",
        ),
    ] {
        let report = check(&source.replace(old, new));
        let [finding] = report.files[1].diagnostics.as_slice() else {
            panic!("{report:#?}")
        };
        assert_eq!(finding.code, "PNX2322", "{report:#?}");
        assert!(finding.message.contains(function), "{finding:#?}");
    }
}

#[test]
fn shadowed_int_predicates_do_not_refine_conditions_or_contracts() {
    let source = r#"(ns adapter (:require [clojure.spec.alpha :as s]))
(defn int? {:t/sig '[string? :-> boolean?]} [x] true)
(s/def ::shadowed int?)
(s/def ::fixed clojure.core/int?)
(defn check {:t/sig '[string? :-> ::fixed]} [x]
  (if (int? x) x 1))
(defn lexical {:t/sig '[(s/fspec :args (s/cat :x string?) :ret boolean?) string? :-> ::fixed]}
  [int? x] (if (int? x) x 1))
"#;
    let report = Session::default().check("adapter.clj", source);
    let codes: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| finding.code.as_str())
        .collect();
    assert_eq!(codes, ["PNX9001", "PNX2322", "PNX2322"], "{report:#?}");
    assert!(
        report.diagnostics[0]
            .message
            .contains("Source predicate int? is unchecked")
    );
}

#[test]
fn checks_nested_lsp_ranges_across_namespaces() {
    let imports = NS.replace(":as-alias", ":as");
    let source = format!(
        r#"{imports}
(defn line-of {{:t/sig '[::wire/range :-> ::wire/line]}} [range]
  (:line (:start range)))
(defn largest {{:t/sig '[:-> ::wire/line]}} []
  (line-of {{:start {{:line 2147483647 :character 0}}
            :end {{:line 0 :character -2147483648}}}}))
(defn checked {{:t/sig '[map? :-> ::wire/range]}} [range]
  (if (s/valid? ::wire/range range)
    range
    (throw (ex-info "invalid range" {{}}))))
"#
    );
    let report = check(&source);
    assert!(report.ok && report.complete, "{report:#?}");
    for input in ["\"42\"", "42.0", "2147483648", "-2147483649", "2147483648N"] {
        let bad = source.replace("2147483647", input);
        let report = check(&bad);
        let [finding] = report.files[1].diagnostics.as_slice() else {
            panic!("{input}: {report:#?}")
        };
        assert_eq!(finding.code, "PNX2322", "{input}: {finding:#?}");
        assert!(
            finding.notes.iter().any(|note| note.contains(":line"))
                || finding.message.contains(":line"),
            "{input}: {finding:#?}"
        );
    }
}

#[test]
fn separates_a_general_integer_input_and_the_converted_output() {
    let source = format!(
        r#"{NS}
(s/def ::alias ::wire/line)
(defn integer-value {{:t/sig '[integer? :-> integer?]}} [x] x)
(defn wide-integer {{:t/sig '[:-> integer?]}} [] (integer-value 9223372036854775808N))
(defn normalize {{:t/sig '[integer? :-> ::wire/line]}} [x] (s/conform ::alias x))
(defn validate {{:t/sig '[integer? :-> ::wire/line]}} [x]
  (if (s/valid? ::alias x) x (throw (ex-info "invalid line" {{}}))))
(defn normalized {{:t/sig '[:-> ::wire/line]}} [] (normalize 42N))
(defn rejected {{:t/sig '[:-> #{{:clojure.spec.alpha/invalid}}]}} []
  (s/conform ::alias "42"))
(defn valid {{:t/sig '[:-> #{{true}}]}} [] (s/valid? ::alias 84/2))
(defn invalid {{:t/sig '[:-> #{{false}}]}} [] (s/valid? ::alias 85/2))
"#
    );
    let report = check(&source);
    assert!(report.ok && report.complete, "{report:#?}");

    let bad = source.replace("(normalize 42N)", "(integer-value 42N)");
    let report = check(&bad);
    let [finding] = report.files[1].diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert!(matches!(finding.code.as_str(), "PNX2322" | "PNX9004"));
    assert!(finding.message.contains("normalized"), "{finding:#?}");
}

#[test]
fn int_overflow_in_conform_and_valid_is_an_exception_not_invalid() {
    let source = format!(
        r#"{NS}
(defn overflow {{:t/sig '[:-> #{{:arithmetic}}]}} []
  (try (s/conform ::wire/line 2147483648)
       (catch ArithmeticException _ :arithmetic)
       (catch Exception _ :wrong-catch)))
(defn underflow {{:t/sig '[:-> #{{:arithmetic}}]}} []
  (try (s/valid? ::wire/line -2147483649N)
       (catch ArithmeticException _ :arithmetic)
       (catch Exception _ :wrong-catch)))
(defn beyond-long {{:t/sig '[:-> #{{:argument}}]}} []
  (try (s/conform ::wire/line 9223372036854775808N)
       (catch IllegalArgumentException _ :argument)
       (catch Exception _ :wrong-catch)))
"#
    );
    let report = check(&source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn a_fixed_width_integer_can_overflow_int_but_fits_long() {
    let imports = NS.replace(":as-alias", ":as");
    let source = format!(
        r#"{imports}
(defn classify {{:t/sig '[int? :-> #{{true :overflow}}]}} [input]
  (try (s/valid? ::wire/line input)
       (catch ArithmeticException _ :overflow)
       (catch IllegalArgumentException _ :out-of-long-range)))"#
    );
    let report = check(&source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn an_arbitrary_width_jvm_integer_can_throw_from_an_overridden_method() {
    let imports = NS.replace(":as-alias", ":as");
    for input in ["integer?", "::wire/line"] {
        let source = format!(
            r#"{imports}
(defn classify {{:t/sig '[{input} :-> #{{true}}]}} [input]
  (try (s/valid? ::wire/line input)
       (catch IllegalStateException _ :custom-method)))"#
        );
        let report = check(&source);
        let [finding] = report.files[1].diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, "PNX2322");
        assert!(finding.message.contains(":custom-method"));
    }
}

#[test]
fn does_not_treat_a_validated_map_shape_as_a_conformed_output() {
    let imports = NS.replace(":as-alias", ":as");
    let source = format!(
        r#"{imports}
(defn conform-range {{:t/sig '[::wire/range :-> ::wire/range]}} [range]
  (s/conform ::wire/range range))
"#
    );
    let report = check(&source);
    assert!(!report.complete, "{report:#?}");
    assert!(
        report.files[1]
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PNX9001"
                && diagnostic
                    .message
                    .contains("closed map with known keyword fields")),
        "{report:#?}"
    );
}

#[test]
fn a_generic_input_is_not_proven_to_fail_validation() {
    let source = format!(
        r#"{NS}
(defn will-fail {{:t/sig '(t/all [a] [a :-> #{{false}}])}} [x]
  (s/valid? ::wire/line x))
"#
    );
    let report = check(&source);
    let [diagnostic] = report.files[1].diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert!(diagnostic.message.contains("will-fail"));
}

#[test]
fn conform_does_not_accept_the_optional_validation_argument() {
    let source = format!(
        r#"{NS}
(defn invalid {{:t/sig '[:-> ::wire/line]}} [] (s/conform ::wire/line 1 nil))
"#
    );
    let report = check(&source);
    assert!(
        report.files[1]
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PNX2554"),
        "{report:#?}"
    );
}

#[test]
fn does_not_confuse_a_shadowed_int_with_a_builtin_conformer() {
    let source = r#"(ns adapter
  (:refer-clojure :exclude [int])
  (:require [clojure.core :as core] [clojure.spec.alpha :as spec]))
(defn int {:t/sig '[integer? :-> integer?]} [x] x)
(spec/def ::unknown (spec/and integer? (spec/conformer int)))
(spec/def ::checked (spec/and core/integer? (spec/conformer core/int)))
(defn checked {:t/sig '[:-> ::checked]} [] (spec/conform ::checked 42))
"#;
    let report = Session::default().check("adapter.clj", source);
    assert!(!report.complete, "{report:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    let at = &source[finding.span.start.offset..finding.span.end.offset];
    assert_eq!(finding.code, "PNX9001", "{finding:#?}");
    assert_eq!(at, "(spec/conformer int)", "{finding:#?}");
}

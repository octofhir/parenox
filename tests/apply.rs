use parenox::{Session, Source};

#[test]
fn applies_str_to_realized_bounded_string_mappings() {
    let source = r#"(ns demo (:refer-clojure :rename {map each partial prefix}))
(defn suffix {:t/sig '[:-> #{" hey  end"}]} []
  (let [args ["hey" nil "end"]]
    (apply str (each (prefix str " ") args))))
(defn plain {:t/sig '[:-> #{"xy"}]} []
  (apply str (clojure.core/map str ["x" nil "y"])))
(defn empty-input {:t/sig '[:-> #{""}]} []
  (apply str (each (prefix str "ignored") nil)))
(defn capture-first {:t/sig '[:-> #{:prefix}]} []
  (try (apply str (each (prefix str (throw (ex-info "prefix" {:stage :prefix})))
                       (throw (ex-info "source" {:stage :source}))))
       (catch clojure.lang.ExceptionInfo error (:stage (ex-data error)))))
"#;
    let report = Session::default().check("demo.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn forced_string_mapping_reports_its_written_source_and_requires_ordinary_traversal() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
(defn invalid {:t/sig '[:-> string?]} []
  (apply str (map (partial str " ") 42)))
(defn custom {:t/sig '[(s/tuple #{"x"} #{"y"}) :-> string?]} [items]
  (apply str (map (partial str " ") items)))
(defn abstract-source {:t/sig '[number? :-> string?]} [items]
  (apply str (map str items)))
(defn exact-value {:t/sig '[integer? :-> string?]} [items]
  (case items 5 (apply str (map str items)) "fallback"))
(defn possible-empty {:t/sig '[boolean? :-> string?]} [empty?]
  (let [items (if empty? nil 42)]
    (apply str (map str items))))
"#;
    let report = Session::default().check("demo.clj", source);
    let mismatches: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.code == "PNX2322")
        .collect();
    let [mismatch] = mismatches.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(
        &source[mismatch.span.start.offset..mismatch.span.end.offset],
        "42"
    );
    assert!(
        report.diagnostics.iter().any(|finding| {
            finding.code == "PNX9001"
                && &source[finding.span.start.offset..finding.span.end.offset] == "items"
        }),
        "{report:#?}"
    );
}

#[test]
fn string_mapping_stops_on_a_proven_capture_failure() {
    let source = r#"(ns demo)
(defn ignores-value {:t/sig '[number? :-> string?]} [_] "prefix")
(defn contract-only {:t/sig '[:-> string?]} []
  (apply str (map (partial str (ignores-value :wrong)) (subs 42 0))))
(defn runtime-failure {:t/sig '[:-> string?]} []
  (apply str (map (partial str (subs false 0)) (subs :unreachable 0))))
"#;
    let report = Session::default().check("demo.clj", source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.code == "PNX2322")
        .map(|finding| &source[finding.span.start.offset..finding.span.end.offset])
        .collect();
    assert_eq!(findings, [":wrong", "42", "false"], "{report:#?}");
}

#[test]
fn spreads_nil_vectors_and_exact_seqs_across_project_namespaces() {
    let input = [
        Source {
            path: "apply.clj",
            source: include_str!("../examples/apply.clj"),
        },
        Source {
            path: "provider.clj",
            source: include_str!("../examples/arities/provider.clj"),
        },
        Source {
            path: "variadic.clj",
            source: include_str!("../examples/variadic.clj"),
        },
        Source {
            path: "apply_bad.clj",
            source: include_str!("../examples/apply_bad.clj"),
        },
    ];
    let report = Session::default().check_project(&input, None);
    assert!(report.complete, "{report:#?}");
    assert!(report.files[..3].iter().all(|file| file.ok), "{report:#?}");
    let [element, arity] = report.files[3].diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(element.code, "PNX2322");
    assert_eq!(
        &input[3].source[element.span.start.offset..element.span.end.offset],
        "\"3\""
    );
    assert_eq!(element.suggestions[0].replacement, "3");
    assert_eq!(arity.code, "PNX2554");
    assert!(arity.message.contains("0 or 1"), "{arity:#?}");
}

#[test]
fn checks_each_possible_exact_shape_and_protects_projected_spans() {
    let source = r#"(ns demo)
        (defn f {:t/sig '[boolean? :-> number?]} [missing?]
          (let [args (if missing? [] ["3"])]
            (apply inc args)))"#;
    let report = Session::default().check("apply.clj", source);
    assert!(report.complete, "{report:#?}");
    let [arity, element] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(arity.code, "PNX2554");
    assert_eq!(element.code, "PNX2322");
    assert!(element.suggestions.is_empty(), "{element:#?}");
    assert!(element.help.as_deref().unwrap().contains("elements"));
}

#[test]
fn diagnoses_nonseqable_inputs_and_marks_open_spread_as_incomplete() {
    let source = r#"(ns demo)
        (defn bad {:t/sig '[:-> number?]} []
          (apply + true))
        (defn open {:t/sig '[(clojure.spec.alpha/coll-of number? :kind vector?) :-> number?]} [xs]
          (apply (fn [& values] (first values)) xs))"#;
    let report = Session::default().check("apply.clj", source);
    assert!(!report.complete, "{report:#?}");
    let [nonseqable, open] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(nonseqable.code, "PNX2322");
    assert_eq!(open.code, "PNX9001");
    assert!(open.message.contains("cannot infer every argument count"));
}

#[test]
fn resolves_apply_and_evaluates_the_target_before_other_inputs() {
    let source = r#"(ns demo)
        (defn f {:t/sig '[:-> #{:target}]} []
          (try
            (apply (throw (ex-info "target" {:which :target}))
                   (throw (ex-info "prefix" {:which :prefix}))
                   (throw (ex-info "spread" {:which :spread})))
            (catch clojure.lang.ExceptionInfo e (:which (ex-data e)))))
        (defn g {:t/sig '[:-> number?]} []
          (let [apply (fn [a b] (+ a b))]
            (apply 1 2)))"#;
    let report = Session::default().check("apply.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn projects_fields_in_fixed_apply_arguments_through_concat() {
    let source = r#"(ns command)
(defn run-command {:t/sig '[string? {:settings {:clean {:automatic boolean?}}} :-> boolean?]}
  [uri db] (get-in db [:settings :clean :automatic]))
(defn directly-written {:t/sig '[:-> boolean?]} []
  (apply run-command "file:///🦊" [{:settings {:clean {:automatic "false"}}}]))
(defn combined {:t/sig '[:-> boolean?]} []
  (let [db {:settings {:clean {:automatic "false"}}}]
    (apply run-command "file:///🦊" (concat nil [db]))))"#;
    let report = Session::default().check("command.clj", source);
    assert!(report.complete, "{report:#?}");
    let [literal, variable] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    let written =
        |finding: &parenox::Diagnostic| &source[finding.span.start.offset..finding.span.end.offset];
    assert_eq!(written(literal), "\"false\"");
    assert_eq!(written(variable), "db");
    assert_eq!(literal.mismatch_path, variable.mismatch_path);
    assert!(literal.message.contains("Argument 2"));
    assert!(
        variable
            .notes
            .iter()
            .any(|note| note.contains(":automatic"))
    );
}

#[test]
fn checks_concat_operands_before_spreading_and_requires_ordinary_vectors() {
    let source = r#"(ns command (:require [clojure.spec.alpha :as s]))
(defn eager {:t/sig '[:-> #{:operand}]} []
  (try
    (apply (fn [_] :callee)
      (concat [:first] (throw (ex-info "late operand" {:part :operand}))))
    (catch clojure.lang.ExceptionInfo error (:part (ex-data error)))))
(defn concatenate {:t/sig '[:-> (s/and int? #{42})]} []
  (apply inc (clojure.core/concat nil [41] [])))
(defn shadowed {:t/sig '[:-> (s/and int? #{42})]} []
  (let [concat (fn [_ _] [41])]
    (apply inc (concat "raw" nil))))
(defn structural {:t/sig '[(s/tuple string?) :-> any?]} [source]
  (apply inc (concat source)))"#;
    let report = Session::default().check("command.clj", source);
    assert!(!report.complete, "{report:#?}");
    let [gap] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(gap.code, "PNX9001");
    assert_eq!(gap.severity, "warning");
    assert_eq!(
        &source[gap.span.start.offset..gap.span.end.offset],
        "source"
    );
}

#[test]
fn forwards_typed_rest_arguments_and_checks_every_reachable_overload() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn sum {:t/sig '[number? & number? :-> number?]} [base & more]
          (apply + base more))
        (defn total {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [values]
          (apply sum 0 values))
        (defn describe {:t/sig '[[number? :-> string?] [number? & number? :-> number?]]}
          ([base] (str base))
          ([base & more] (apply + base more)))
        (defn from-nonempty {:t/sig '[(s/coll-of number? :kind vector?)
                                     :-> (s/nilable (s/or :label string? :total number?))]}
          [values]
          (when-let [items (seq values)] (apply describe items)))
    "#;
    let report = Session::default().check("forward.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let app = r#"(ns app (:require [clojure.spec.alpha :as s]))
        (defn total {:t/sig '[(s/coll-of string? :kind vector?) :-> number?]}
          [values] (apply db/sum 0 values))"#;
    let report = Session::default().check_project(
        &[Source {
            path: "app.clj",
            source: app,
        }],
        Some(Source {
            path: "externs.edn",
            source: "{db/sum [number? & number? :-> number?]}",
        }),
    );
    assert!(report.complete, "{report:#?}");
    let [diagnostic] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(
        diagnostic.message,
        "apply collection elements: expected Number, got String"
    );
    assert_eq!(
        &app[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "values"
    );
    assert!(diagnostic.suggestions.is_empty());
}

#[test]
fn open_spread_reports_a_possible_arity_and_exact_literals_stay_exact() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn fixed {:t/sig '[number? :-> number?]} [x] x)
        (defn maybe {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [values]
          (apply fixed values))
        (defn nonempty {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [values]
          (if-let [items (seq values)] (apply fixed items) 0))
        (defn minus {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [values]
          (apply - values))"#;
    let report = Session::default().check("arity.clj", source);
    assert!(report.complete, "{report:#?}");
    let counts: Vec<_> = report
        .diagnostics
        .iter()
        .map(|d| (d.code.as_str(), d.message.as_str()))
        .collect();
    assert_eq!(
        counts,
        [
            (
                "PNX2554",
                "apply may pass 0 arguments, but the function expects 1"
            ),
            (
                "PNX2554",
                "apply may pass 2 arguments, but the function expects 1"
            ),
            (
                "PNX2554",
                "apply may pass 0 arguments, but the function expects at least 1"
            ),
        ]
    );

    let source = format!(
        "(ns demo) (defn f {{:t/sig '[:-> number?]}} []
           (apply + [{} \"3\"]))",
        "1 ".repeat(40)
    );
    let report = Session::default().check("literal.clj", &source);
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(
        &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "\"3\""
    );
    assert_eq!(diagnostic.suggestions[0].replacement, "3");
}

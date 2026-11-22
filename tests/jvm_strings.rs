use parenox::{DocumentHover, Session, Source};

const SOURCE: &str = include_str!("../examples/jvm_strings.cljc");

#[test]
fn unmodeled_jvm_forms_keep_their_syntax_and_eager_operand_resolution() {
    let source = r#"(ns demo)
(defn member {:t/sig '[t/any :-> t/any]} [file] (.getName file))
(defn nested {:t/sig '[t/any :-> t/any]} [file] (. file (getName) ignored))
(defn constructor {:t/sig '[:-> t/any]} [] (java.io.File. "document.txt"))
(defn new-form {:t/sig '[:-> t/any]} [] (new java.io.File unknown-file))
(defn static-field {:t/sig '[:-> t/any]} [] (. java.io.File separator))
(defn malformed {:t/sig '[:-> t/any]} [] (.getName))
(defn malformed-new {:t/sig '[:-> t/any]} [] (new "java.io.File"))
(defn nested-length {:t/sig '[:-> #{:match}]} []
  (if (= 4 (. "text" (length) ignored)) :match nil))
"#;
    let report = Session::default().check("jvm.clj", source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|d| {
            (
                d.code.as_str(),
                &source[d.span.start.offset..d.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX9001", "(.getName file)"),
            ("PNX9001", "(. file (getName) ignored)"),
            ("PNX9001", "(java.io.File. \"document.txt\")"),
            ("PNX9001", "(new java.io.File unknown-file)"),
            ("PNX2304", "unknown-file"),
            ("PNX9001", "(. java.io.File separator)"),
            ("PNX1001", "(.getName)"),
            ("PNX1001", "(new \"java.io.File\")"),
        ],
        "{report:#?}"
    );
}

#[test]
fn a_resolved_macro_takes_precedence_over_dotted_member_syntax() {
    let source = r#"(ns demo)
(defmacro .length [_] :from-macro)
(defn explicit {:t/sig '[:-> t/any]} [] (.length ignored))
(defn stock {:t/sig '[t/any :-> t/any]} [file] (.. file getName))
"#;
    let report = Session::default().check("macros.clj", source);
    let [declaration, custom, stock] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(declaration.code, "PNX9001");
    for (finding, operator, name) in [
        (custom, ".length", ".length"),
        (stock, "..", "clojure.core/.."),
    ] {
        assert_eq!(finding.code, "PNX9001");
        assert_eq!(
            finding.message,
            format!("Macro {name} has no checked scope")
        );
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            operator
        );
    }
}

#[test]
fn imported_class_values_follow_lexical_resolution_and_import_edits() {
    let source = r#"(ns demo (:import [java.io File]))
(defn imported {:t/sig '[:-> t/any]} [] File)
(defn implicit {:t/sig '[:-> t/any]} [] String)
(defn shadowed {:t/sig '[:-> number?]} [] (let [File 1] File))
(defn typo {:t/sig '[:-> t/any]} [] Fiel)
"#;
    let mut session = Session::default();
    let original = session.check("jvm.clj", source);
    let findings: Vec<_> = original
        .diagnostics
        .iter()
        .map(|d| {
            (
                d.code.as_str(),
                &source[d.span.start.offset..d.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX9001", "File"),
            ("PNX9001", "String"),
            ("PNX2304", "Fiel")
        ],
        "{original:#?}"
    );

    let edited = source.replace(" (:import [java.io File])", "");
    let warm = session.check("jvm.clj", &edited);
    let cold = Session::default().check("jvm.clj", &edited);
    let normalize = |report: &parenox::Report| serde_json::to_value(&report.diagnostics).unwrap();
    assert_eq!(warm.diagnostics[0].code, "PNX2304", "{warm:#?}");
    assert_eq!(normalize(&warm), normalize(&cold));
    assert_eq!(
        normalize(&session.check("jvm.clj", source)),
        normalize(&original)
    );
}

#[test]
fn ordinary_string_calls_check_arity_only_after_eager_arguments_can_return() {
    let source = r#"(ns demo (:require [clojure.string :as str]))
(defn dead-call {:t/sig '[:-> string?]} []
  (do (subs nil 0) (str/upper-case)))
(defn dead-arguments {:t/sig '[:-> string?]} []
  (str/upper-case (subs nil 0) missing-extra))
(defn reached-call {:t/sig '[:-> string?]} []
  (str/upper-case))
(defn inlined-call {:t/sig '[:-> number?]} []
  (do (subs nil 0) (inc)))
"#;
    let report = Session::default().check("demo.clj", source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX2322", "nil"),
            ("PNX2322", "nil"),
            ("PNX2304", "missing-extra"),
            ("PNX2554", "(str/upper-case)"),
            ("PNX2322", "nil"),
            ("PNX2554", "(inc)"),
        ],
        "{report:#?}"
    );
}

#[test]
fn eager_vector_inputs_stop_only_after_a_proven_runtime_failure() {
    let source = r#"(ns demo)
(defn accepts-at-runtime {:t/sig '[string? :-> string?]} [_] "a")
(defn split-text {:t/sig '[:-> string?]} []
  (let [[prefix suffix] [(subs nil 0 1) (subs "abcd" -2)]]
    (str prefix suffix)))
(defn compile-tail {:t/sig '[:-> t/any]} []
  [(subs false 0) (subs "abcd" missing-offset)])
(defn apply-spread {:t/sig '[:-> string?]} []
  (apply str [(subs :bad-text 0) (subs "abcd" -3)]))
(defn apply-prefix {:t/sig '[:-> string?]} []
  (apply str (subs :bad-prefix 0) [(subs :skipped 0)]))
(defn apply-target {:t/sig '[:-> string?]} []
  (apply (subs :bad-target 0) [(subs :skipped 0)]))
(defn preserves-possible-continuation {:t/sig '[:-> t/any]} []
  [(accepts-at-runtime nil) (subs "abcd" -4)])
(defn apply-captured {:t/sig '[:-> string?]} []
  (apply str (map (partial str (subs true 0) (subs :skipped 0)) ["tail"])))
(defn partial-target {:t/sig '[:-> string?]} []
  ((partial (subs :bad-partial 0) (subs :skipped 0)) "tail"))
"#;
    let report = Session::default().check("demo.clj", source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| matches!(finding.code.as_str(), "PNX2304" | "PNX2322" | "PNX2801"))
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX2322", "nil"),
            ("PNX2322", "false"),
            ("PNX2304", "missing-offset"),
            ("PNX2322", ":bad-text"),
            ("PNX2322", ":bad-prefix"),
            ("PNX2322", ":bad-target"),
            ("PNX2322", "nil"),
            ("PNX2801", "-4"),
            ("PNX2322", "true"),
            ("PNX2322", ":bad-partial"),
        ],
        "{report:#?}"
    );
}

#[test]
fn branch_joins_preserve_normal_results_and_proven_stops() {
    let source = r#"(ns demo)
(defn joined {:t/sig '[boolean? :-> #{"label"}]} [choice]
  (if choice (subs nil 0) "label"))
(defn wrapped {:t/sig '[:-> t/any]} []
  [(if true (subs false 0) "unused") (subs "text" -1)])
(defn wrapped-dispatch {:t/sig '[:-> t/any]} []
  [(condp = :ready :ready (subs :bad-arm 0) "unused") (subs "text" -2)])
(defn if-test {:t/sig '[:-> t/any]} []
  (if (subs :bad-if 0) (subs :unused 0) missing-name))
(defn cond-test {:t/sig '[:-> t/any]} []
  (cond (subs :bad-cond 0) (subs :unused 0) :else (subs :unused 0)))
(defn case-test {:t/sig '[:-> t/any]} []
  (case (subs :bad-case 0) "x" (subs :unused 0) (subs :unused 0)))
(defn or-test {:t/sig '[:-> t/any]} []
  (or (subs :bad-or 0) (subs :unused 0) missing-or))
(defn and-test {:t/sig '[:-> t/any]} []
  (and (subs :bad-and 0) (subs :unused 0)))
(defn if-let-test {:t/sig '[:-> t/any]} []
  (if-let [text (subs :bad-init 0)] (subs :unused 0) missing-init))
(defn accepts-at-runtime {:t/sig '[string? :-> string?]} [_] "prefix")
(defn contract-only {:t/sig '[:-> t/any]} []
  [(if true (accepts-at-runtime nil) "unused") (subs "text" -9)])
"#;
    let report = Session::default().check("demo.clj", source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX2322", "nil"),
            ("PNX2322", "false"),
            ("PNX2322", ":bad-arm"),
            ("PNX2322", ":bad-if"),
            ("PNX2304", "missing-name"),
            ("PNX2322", ":bad-cond"),
            ("PNX2322", ":bad-case"),
            ("PNX2322", ":bad-or"),
            ("PNX2304", "missing-or"),
            ("PNX2322", ":bad-and"),
            ("PNX2322", ":bad-init"),
            ("PNX2304", "missing-init"),
            ("PNX2322", "nil"),
            ("PNX2801", "-9"),
        ],
        "{report:#?}"
    );
}

#[test]
fn sequential_bodies_keep_compiler_checks_and_skip_runtime_suffixes() {
    let source = r#"(ns demo)
(defn explicit {:t/sig '[:-> t/any]} []
  (do (subs :bad-do 0) (subs :dead-do 0) missing-name))
(defn implicit {:t/sig '[:-> t/any]} []
  (subs :bad-body 0)
  (subs :dead-body 0))
(defn caught {:t/sig '[:-> #{:caught}]} []
  (try (subs :bad-try 0) (subs :dead-try 0)
       (catch Throwable _ :caught)))
(defn cleanup {:t/sig '[:-> t/any]} []
  (inc (try :normal (finally (subs :bad-finally 0)))))
(defn assertions {:t/sig '[:-> t/any]} []
  {:post [(subs :dead-post 0) missing-post]}
  (subs :bad-post 0))
(defn accepts-at-runtime {:t/sig '[string? :-> string?]} [_] "ok")
(defn contract-only {:t/sig '[:-> t/any]} []
  (do (accepts-at-runtime false) (subs "abc" -7)))
"#;
    let report = Session::default().check("demo.clj", source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX2322", ":bad-do"),
            ("PNX2304", "missing-name"),
            ("PNX2322", ":bad-body"),
            ("PNX2322", ":bad-try"),
            ("PNX2322", ":bad-finally"),
            ("PNX2304", "missing-post"),
            ("PNX2322", ":bad-post"),
            ("PNX2322", "false"),
            ("PNX2801", "-7"),
        ],
        "{report:#?}"
    );
}

#[test]
fn let_stops_after_failed_initializers_and_eager_defaults() {
    let source = r#"(ns demo)
(defn initializer {:t/sig '[:-> t/any]} []
  (let [stopped (subs :bad-init 0)
        later (subs :dead-init 0)
        callback (fn [_] missing-closure-name)]
    (str later missing-body-name)))
(defn direct-default {:t/sig '[:-> t/any]} []
  (let [{:keys [left right]
         :or {left (subs :bad-default 0)
              right (subs :dead-default 0)}}
        (subs :dead-map-init 0)]
    (subs :dead-map-body 0)))
(defn accepts-at-runtime {:t/sig '[string? :-> any?]} [_] :default)
(defn contract-only {:t/sig '[:-> t/any]} []
  (let [{:keys [left right]
         :or {left (accepts-at-runtime false) right (subs "abc" -5)}}
        {:left :present}]
    [left right]))
"#;
    let report = Session::default().check("demo.clj", source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX2322", ":bad-init"),
            ("PNX2304", "missing-closure-name"),
            ("PNX2304", "missing-body-name"),
            ("PNX2322", ":bad-default"),
            ("PNX2322", "false"),
            ("PNX2801", "-5"),
        ],
        "{report:#?}"
    );
}

#[test]
fn eager_calls_stop_after_a_failed_argument_or_target() {
    let source = r#"(ns demo)
(defn accepts-two {:t/sig '[t/any t/any :-> t/any]} [a b] [a b])
(defn builtin {:t/sig '[:-> t/any]} []
  (str (subs :bad-first 0) (subs :dead-next 0) missing-argument))
(defn typed {:t/sig '[:-> t/any]} []
  (accepts-two (subs :bad-typed 0) (subs :dead-typed 0)))
(defn wrong-arity {:t/sig '[:-> t/any]} []
  (accepts-two (subs :bad-arity 0)))
(defn computed {:t/sig '[:-> t/any]} []
  ((subs :bad-target 0) (subs :dead-target 0)))
(defn host {:t/sig '[:-> t/any]} []
  (.startsWith ^String (subs :bad-receiver 0) (subs :dead-member 0)))
(defn nil-receiver {:t/sig '[:-> t/any]} []
  (let [^String text nil]
    (.startsWith text (subs :bad-member-arg 0))))
(defn accepts-at-runtime {:t/sig '[string? :-> t/any]} [_] "ok")
(defn contract-only {:t/sig '[:-> t/any]} []
  (str (accepts-at-runtime false) (subs "abc" -4)))
"#;
    let report = Session::default().check("demo.clj", source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX2322", ":bad-first"),
            ("PNX2304", "missing-argument"),
            ("PNX2322", ":bad-typed"),
            ("PNX2322", ":bad-arity"),
            ("PNX2322", ":bad-target"),
            ("PNX2322", ":bad-receiver"),
            ("PNX2322", ":bad-member-arg"),
            ("PNX2322", "false"),
            ("PNX2801", "-4"),
        ],
        "{report:#?}"
    );
}

#[test]
fn an_unresolved_operator_does_not_prove_an_argument_is_evaluated() {
    let source = r#"(ns demo)
(defn from-opaque-macro {:t/sig '[:-> string?]} []
  (let [continuation "after"]
    (opaque.macros/maybe-discard (str (subs nil 0)) (subs "unused" -6))
    (opaque.macros/maybe-discard (throw (ex-info "unused" {})) :unused)
    continuation))
"#;
    let mut session = Session::default();
    let report = session.check("demo.clj", source);
    assert!(report.diagnostics.iter().any(|finding| {
        finding.code == "PNX2801"
            && &source[finding.span.start.offset..finding.span.end.offset] == "-6"
    }));
    let offset = source.rfind("continuation").unwrap();
    let Some(DocumentHover::Local(hover)) = session.document_hover_at("demo.clj", offset) else {
        panic!("an unresolved operator can discard the failing form");
    };
    assert_eq!(hover.inferred_type, "\"after\"");
}

#[test]
fn literal_function_metadata_runs_at_preparation_and_let_binding() {
    let source = r#"(ns demo)
(defn direct {:t/sig '[:-> t/any]} []
  (^{:label (subs :bad-direct 0)}
   (fn self [value] (self value) missing-direct)
   (subs :dead-direct-argument 0)))
(defn apply-target {:t/sig '[:-> t/any]} []
  (apply ^{:label (subs :bad-apply 0)}
         #(str % missing-apply)
         [(subs :dead-apply-argument 0)]))
(defn bound {:t/sig '[:-> t/any]} []
  (let [callback ^{:label (subs :bad-bind 0)} (fn [value] (str value missing-bound))
        late (subs :dead-initializer 0)]
    (callback late missing-argument)))
(defn formed {:t/sig '[:-> #{"word!"}]} []
  (apply ^{:label :present} #(str % "!") ["word"]))
"#;
    let report = Session::default().check("demo.clj", source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX2322", ":bad-direct"),
            ("PNX2304", "missing-direct"),
            ("PNX2322", ":bad-apply"),
            ("PNX2304", "missing-apply"),
            ("PNX2322", ":bad-bind"),
            ("PNX2304", "missing-bound"),
            ("PNX2304", "missing-argument"),
        ],
        "{report:#?}"
    );
}

#[test]
fn failed_map_operands_stop_before_preparing_updater_metadata() {
    let source = r#"(ns demo)
(defn map-target {:t/sig '[:-> t/any]} []
  (update-vals (subs :bad-target 0)
               ^{:label (subs :dead-transform-meta 0)} #(str % missing-body)))
(defn key-expression {:t/sig '[:-> t/any]} []
  (update {:value "x"} (subs :bad-key 0)
          ^{:label (subs :dead-update-meta 0)} #(str %)))
(defn path-formation {:t/sig '[:-> t/any]} []
  (update-in {:value "x"} ^{:label (subs :bad-path-meta 0)} [:value]
             ^{:label (subs :dead-update-in-meta 0)} #(str %)))
(defn inline-path {:t/sig '[:-> #{"x"}]} []
  (get-in {:value "x"} ^{:label (subs :ignored-inline-meta 0)} [:value]))
(defn eager-path {:t/sig '[:-> t/any]} []
  (get-in {:value "x"} ^{:label (subs :bad-eager-meta 0)} [:value] "fallback"))
"#;
    let report = Session::default().check("demo.clj", source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX2322", ":bad-target"),
            ("PNX2304", "missing-body"),
            ("PNX2322", ":bad-key"),
            ("PNX2322", ":bad-path-meta"),
            ("PNX2322", ":bad-eager-meta"),
        ],
        "{report:#?}"
    );
}

#[test]
fn vector_metadata_effects_follow_elements_and_merge_before_checking() {
    let source = r#"(ns demo (:require [clojure.string :as string]) (:import java.util.UUID))
(defn inert-class {:t/sig '[:-> #{"ab"}]} []
  (apply str ^String ^:checked ["a" "b"]))
(defn imported-class {:t/sig '[:-> #{"ab"}]} []
  (apply str ^UUID ["a" "b"]))
(defn qualified-class {:t/sig '[:-> #{"ab"}]} []
  (apply str ^java.util.Date ["a" "b"]))
(defn default-classes {:t/sig '[:-> #{"ab"}]} []
  (apply str ^{:first Exception :second Runnable} ["a" "b"]))
(defn core-function {:t/sig '[:-> #{"ab"}]} []
  (apply str ^long ["a" "b"]))
(defn specialized-core-functions {:t/sig '[:-> #{"ab"}]} []
  (apply str ^{:substring subs :transform mapv :spread apply :divide /} ["a" "b"]))
(defn locally-shadowed {:t/sig '[:-> #{"ab"}]} []
  (let [String :local] (apply str ^String ["a" "b"])))
(defn overridden {:t/sig '[:-> #{"ab"}]} []
  (apply str ^{::string/stamp :override} ^{:clojure.string/stamp missing-before-merge} ["a" "b"]))
(defn effect-after-elements {:t/sig '[:-> #{:metadata}]} []
  (try (apply str ^{:stamp (throw (ex-info "stamp" {:stage :metadata}))} ["a" "b"])
       (catch clojure.lang.ExceptionInfo error (:stage (ex-data error)))))
(defn first-element-fails {:t/sig '[:-> #{:element}]} []
  (try ^{:stamp (throw (ex-info "stamp" {:stage :metadata}))}
       [(throw (ex-info "element" {:stage :element}))]
       (catch clojure.lang.ExceptionInfo error (:stage (ex-data error)))))
(defn metadata-order {:t/sig '[:-> #{:inner}]} []
  (try ^{:outer (throw (ex-info "outer" {:stage :outer}))}
       ^{:inner (throw (ex-info "inner" {:stage :inner}))} []
       (catch clojure.lang.ExceptionInfo error (:stage (ex-data error)))))
"#;
    let report = Session::default().check("demo.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn vector_metadata_still_resolves_names_after_element_failure() {
    let source = r#"(ns demo (:require [clojure.string :as string]))
(defn broken {:t/sig '[:-> t/any]} []
  (apply str ^{:stamp missing-metadata} [(subs false 0)]))
(defn unchecked {:t/sig '[:-> t/any]} []
  ^SomeRuntimeClass [])
"#;
    let report = Session::default().check("demo.clj", source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX2304", "missing-metadata"),
            ("PNX2322", "false"),
            ("PNX9001", "SomeRuntimeClass"),
        ]
    );
}

#[test]
fn opaque_vector_metadata_withholds_captured_function_hover() {
    let source = r#"(ns demo)
(defn captured {:t/sig '[:-> t/any]} []
  (let [saved (fn [item] item)
        before (saved "first")]
    ^{(identity :stamp) (saved 7)} []))
"#;
    let mut session = Session::default();
    let report = session.check("demo.clj", source);
    let [gap] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!((gap.code.as_str(), gap.severity), ("PNX9001", "warning"));
    let offset = source.find("] item").unwrap() + "] ".len();
    assert!(session.document_hover_at("demo.clj", offset).is_none());

    let source = source.replace("(identity :stamp)", ":stamp");
    let report = session.check("demo.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");
    let Some(DocumentHover::Local(hover)) = session.document_hover_at("demo.clj", offset) else {
        panic!("missing hover for the evaluated metadata call");
    };
    assert_eq!(hover.inferred_type, "7 | \"first\"");
}

#[test]
fn join_accepts_seqable_values_and_reports_the_invalid_collection_input() {
    let source = r#"(ns labels (:require [clojure.string :as str] [clojure.spec.alpha :as s]))
        (defn entries {:t/sig '[map? :-> string?]} [m] (str/join :entry m))
        (defn characters {:t/sig '[string? :-> string?]} [text] (str/join \/ text))
        (defn no-items {:t/sig '[:-> string?]} [] (str/join nil))
        (defn maybe-items {:t/sig '[(s/nilable number?) :-> string?]} [value] (str/join value))
        (defn without-types [] (str/join \/ 42))"#;
    let report = Session::default().check("labels.cljc", source);
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == "error")
        .collect();
    assert_eq!(errors.len(), 2, "{report:#?}");
    for (error, target) in errors.into_iter().zip(["value", "42"]) {
        assert_eq!(error.code, "PNX2322");
        assert_eq!(error.expected.as_deref(), Some("Seqable or Nil"));
        assert_eq!(error.actual.as_deref(), Some("Number"));
        assert_eq!(error.contract.as_ref().unwrap().name, "clojure.string/join");
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            target
        );
    }
}

#[test]
fn char_at_checks_utf16_bounds_in_unannotated_bodies() {
    let source = r#"(ns members)
        (defn exact {:t/sig '[:-> #{\a}]} [] (. "😀a" (charAt 2)))
        (defn surrogate {:t/sig '[:-> char?]} [] (.charAt "😀" 0))
        (defn negative {:t/sig '[string? :-> char?]} [text] (.charAt text -1))
        (defn outside [] (long (.charAt "😀" 2)))
        (defn guarded [arg] (when arg (.charAt arg 2)))"#;
    let report = Session::default().check("members.cljc", source);
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == "error")
        .collect();
    assert_eq!(errors.len(), 2, "{report:#?}");
    for (error, text) in errors.into_iter().zip(["-1", "2"]) {
        assert_eq!(error.code, "PNX2801");
        assert_eq!(
            error.contract.as_ref().unwrap().name,
            "java.lang.String/charAt"
        );
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            text
        );
    }
}

#[test]
fn substring_uses_utf16_bounds_and_marks_the_invalid_index() {
    let source = r#"(ns captures)
        (defn suffix {:t/sig '[:-> #{"a"}]} [] (subs "😀a" 2))
        (defn prefix {:t/sig '[:-> #{"😀"}]} [] (subs "😀a" 0 2))
        (defn half {:t/sig '[:-> string?]} [] (subs "😀a" 0 1))"#;
    let mut session = Session::default();
    let report = session.check("members.cljc", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let invalid = r#"(ns captures (:refer-clojure :rename {subs piece}))
        (defn outside [] (piece "😀" 3))
        (defn reversed [text] (piece text 4 2))
        (defn wrong [] (piece "abc" "0"))"#;
    let report = session.check("members.cljc", invalid);
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.severity == "error")
        .collect();
    assert_eq!(errors.len(), 3, "{report:#?}");
    for (finding, (code, target)) in
        errors
            .into_iter()
            .zip([("PNX2801", "3"), ("PNX2801", "2"), ("PNX2322", "\"0\"")])
    {
        assert_eq!(finding.code, code);
        assert_eq!(
            &invalid[finding.span.start.offset..finding.span.end.offset],
            target
        );
    }
}

#[test]
fn rejected_substring_operands_do_not_supply_a_result_or_a_bounds_error() {
    let source = r#"(ns captures)
        (defn invalid-text {:t/sig '[:-> t/any]} [] (inc (subs nil -1)))
        (defn invalid-index {:t/sig '[:-> t/any]} [] (inc (subs "abc" \a)))
        (defn nested-call {:t/sig '[:-> t/any]} []
          (inc (subs (subs nil 0) :bad-index)))
        (defn partial-text {:t/sig '[boolean? :-> t/any]} [flag]
          (inc (subs (if flag "abc" nil) 0)))"#;
    let mut session = Session::default();
    let report = session.check("members.cljc", source);
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.severity == "error")
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        errors,
        [
            ("PNX2322", "nil"),
            ("PNX2322", "\\a"),
            ("PNX2322", "nil"),
            ("PNX2322", "(subs (if flag \"abc\" nil) 0)"),
            ("PNX2322", "(if flag \"abc\" nil)"),
        ],
        "{report:#?}"
    );
}

#[test]
fn a_rejected_substring_chain_keeps_assertion_coverage() {
    let source = r#"(ns captures (:require [clojure.test :refer [deftest is]]))
        (deftest bad-substring (is (= "a" (subs (subs false 0) 1))))"#;
    let report = Session::default().check("members.clj", source);
    assert!(report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX2322");
}

#[test]
fn unannotated_substring_checks_a_locally_computed_utf16_endpoint() {
    let source = r#"
(ns arglist)
(defn contents []
  (let [fn-def {:arglist-strs ["[🦊]"]}
        raw-arglist (get (:arglist-strs fn-def) 0)]
    (subs raw-arglist 1 (dec (count raw-arglist)))))
(defn shadowed [count]
  (subs "x" 1 (dec (count "x"))))
(defn dead []
  (if false (subs "x" 1 (dec (count "x"))) :skip))
"#;
    let broken = source.replace("[🦊]", "x");
    let repaired = source.replace("[🦊]", "[]");
    let mut session = Session::default();
    for (text, expected) in [(source, 0), (&broken, 1), (&repaired, 0), (source, 0)] {
        let report = session.check("arglist.clj", text);
        assert!(!report.complete, "{report:#?}");
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|finding| finding.severity == "error")
            .collect();
        assert_eq!(errors.len(), expected, "{report:#?}");
        if let [finding] = errors.as_slice() {
            assert_eq!(finding.code, "PNX2801");
            assert_eq!(finding.contract.as_ref().unwrap().name, "clojure.core/subs");
            assert_eq!(
                &text[finding.span.start.offset..finding.span.end.offset],
                "(dec (count raw-arglist))"
            );
        }
    }
}

#[test]
fn unannotated_arglist_reads_keep_ordinary_vector_elements() {
    let source = r#"
(ns demo (:require [arglists :as provider]))
(defn local-arglist []
  (let [fn-def {:arglist-strs (first [["[🦊]"]])}
        raw-arglist (first (:arglist-strs fn-def))
        arglist-contents (subs raw-arglist 1 (dec (count raw-arglist)))]
    (re-matches #".*[\[\{].*" arglist-contents)))
(defn structural-arglist []
  (let [raw-arglist (first (provider/arglists))]
    (subs raw-arglist 1 (dec (count raw-arglist)))))
(defn shadowed [first]
  (let [raw-arglist (first ["x"])]
    (subs raw-arglist 1 (dec (count raw-arglist)))))
"#;
    let externs = Source {
        path: "externs.edn",
        source: r#"{arglists/arglists [:-> (clojure.spec.alpha/tuple #{"x"})]}"#,
    };
    let broken = source.replace("[🦊]", "x");
    let mut session = Session::default();
    for (source, expected) in [(source, 0), (broken.as_str(), 1), (source, 0)] {
        let report = session.check_project(
            &[Source {
                path: "src/demo.clj",
                source,
            }],
            Some(externs),
        );
        assert!(!report.complete, "{report:#?}");
        let errors: Vec<_> = report.files[0]
            .diagnostics
            .iter()
            .filter(|finding| finding.severity == "error")
            .collect();
        assert_eq!(errors.len(), expected, "{report:#?}");
        if let [finding] = errors.as_slice() {
            assert_eq!(finding.code, "PNX2801");
            assert_eq!(finding.span.start.line, 6);
            assert_eq!(finding.contract.as_ref().unwrap().name, "clojure.core/subs");
            assert_eq!(
                &source[finding.span.start.offset..finding.span.end.offset],
                "(dec (count raw-arglist))"
            );
        }
    }
}

#[test]
fn unannotated_line_slice_checks_the_first_invalid_bound() {
    let source = r#"
(defn alias-text []
  (let [lines (nth [["a🦊b"]] (dec 1))
        name-end-row 1
        name-end-col 2
        alias-col 5]
    (subs (nth lines (dec name-end-row))
          (dec name-end-col) (dec alias-col))))
(defn index-only [] (nth [] 3))
"#;
    let broken_column = source.replace("alias-col 5", "alias-col 6");
    let broken_row = broken_column.replace("name-end-row 1", "name-end-row 2");
    let mut session = Session::default();
    for (text, expected) in [
        (source, None),
        (
            &broken_column,
            Some(("clojure.core/subs", "(dec alias-col)")),
        ),
        (
            &broken_row,
            Some(("clojure.core/nth", "(dec name-end-row)")),
        ),
        (source, None),
    ] {
        let report = session.check("aliases.clj", text);
        assert!(!report.complete, "{report:#?}");
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|finding| finding.severity == "error")
            .collect();
        match (errors.as_slice(), expected) {
            ([], None) => {}
            ([finding], Some((origin, fragment))) => {
                assert_eq!(finding.code, "PNX2801");
                assert_eq!(finding.contract.as_ref().unwrap().name, origin);
                assert_eq!(
                    &text[finding.span.start.offset..finding.span.end.offset],
                    fragment
                );
            }
            _ => panic!("{report:#?}"),
        }
    }
}

#[test]
fn multiline_alias_text_checks_the_middle_slice_before_the_last_line() {
    let source = r#"(ns aliases (:require [clojure.string :as string]))
(defn between []
  (let [lines ["name" "  :as-alias" "  ; 🦊" "alias"]
        name-end-row 1 name-end-col 5 alias-row 4 alias-col 3]
    (str
      (subs (nth lines (dec name-end-row)) (dec name-end-col))
      "\n"
      (string/join "\n" (subvec lines name-end-row (dec alias-row)))
      "\n"
      (subs (nth lines (dec alias-row)) 0 (dec alias-col)))))"#;
    let broken = source.replace("alias-row 4", "alias-row 6");
    let mut session = Session::default();
    for (text, expected) in [(source, 0), (broken.as_str(), 1), (source, 0)] {
        let report = session.check("aliases.clj", text);
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|finding| finding.severity == "error")
            .collect();
        assert_eq!(errors.len(), expected, "{report:#?}");
        if let [finding] = errors.as_slice() {
            assert_eq!(finding.code, "PNX2801");
            assert_eq!(
                finding.contract.as_ref().unwrap().name,
                "clojure.core/subvec"
            );
            assert_eq!(
                &text[finding.span.start.offset..finding.span.end.offset],
                "(dec alias-row)"
            );
            assert_eq!(finding.span.start.line, 8);
        }
    }
}

#[test]
fn literal_rendering_checks_version_labels_and_nil_without_a_catch_path() {
    let source = r#"
(defn version-label {:t/sig '[:-> #{"2026.11-SNAPSHOT (🦊12345)"}]} []
  (let [version "2026.11-SNAPSHOT" revision "🦊123456789"]
    (str version " (" (subs revision 0 (min 7 (count revision))) ")")))
(defn pure {:t/sig '[:-> #{"a🦊"}]} []
  (try (str "a" nil "🦊") (catch Throwable _ :render-failed)))
(defn empty-label {:t/sig '[:-> #{""}]} [] (str))
(defn fixed-spread {:t/sig '[:-> #{"a🦊"}]} [] (apply str ["a" nil "🦊"]))
"#;
    let report = Session::default().check("labels.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn rendering_caps_literals_and_preserves_effects_of_unknown_arguments() {
    let astral = "🦊".repeat(256);
    let source = format!(
        r#"(ns labels (:require [clojure.spec.alpha :as s]))
(defn boundary {{:t/sig '[:-> string?]}} []
  (let [boundary (str "{astral}" nil)] boundary))
(defn over-limit {{:t/sig '[:-> string?]}} []
  (let [over-limit (str "{astral}" "a")] over-limit))
(defn uncertain {{:t/sig '[any? :-> any?]}} [object]
  (let [uncertain (try (str "{astral}" "a" object)
                       (catch Throwable _ :render-failed))] uncertain))
(defn open-spread {{:t/sig '[(s/coll-of #{{"a"}} :kind vector?) :-> string?]}} [pieces]
  (let [joined (apply str pieces)] joined))"#,
    );
    let mut session = Session::default();
    let report = session.check("render.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");
    for (marker, expected) in [
        ("boundary))", format!("\"{astral}\"")),
        ("over-limit))", "String".into()),
        ("uncertain))", "String | :render-failed".into()),
        ("joined))", "String".into()),
    ] {
        let offset = source.find(marker).unwrap();
        let Some(DocumentHover::Local(hover)) = session.document_hover_at("render.clj", offset)
        else {
            panic!("missing hover for {marker}");
        };
        assert_eq!(hover.inferred_type, expected);
    }
}

#[test]
fn structural_traversal_does_not_supply_literal_str_arguments() {
    let source = r#"(ns rendering (:require [clojure.spec.alpha :as s]))
(defn projected {:t/sig '[(s/tuple #{"x"}) :-> any?]} [parts]
  (let [joined (apply str parts)
        seq-joined (apply str (seq parts))
        prefixed (apply (partial str "prefix:") parts)
        mapped (mapv (fn [part] (str part "!")) parts)]
    [joined seq-joined prefixed mapped]))"#;
    let mut session = Session::default();
    let report = session.check("render.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 4, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .all(|finding| finding.code == "PNX9001" && finding.severity == "warning")
    );
    let start = source.find("[joined seq-joined").unwrap();
    for name in ["joined", "seq-joined", "prefixed", "mapped"] {
        let offset = start + source[start..].find(name).unwrap();
        let hover = session.document_hover_at("render.clj", offset);
        if name == "mapped" {
            let Some(DocumentHover::Local(hover)) = hover else {
                panic!("missing vector result");
            };
            assert_eq!(hover.inferred_type, "Vector<Unknown>");
        } else {
            assert!(hover.is_none(), "unexpected hover for {name}");
        }
    }
    assert!(
        session
            .document_hover_at("render.clj", source.find("part \"!\"").unwrap())
            .is_none()
    );
}

#[test]
fn checks_jvm_utf16_length_and_characters_after_nil_guards() {
    let mut session = Session::default();
    let report = session.check("jvm_strings.cljc", SOURCE);
    assert!(report.ok && report.complete, "{report:#?}");
    let editable = SOURCE.replace("(when name", "(do");
    let report = session.check("jvm_strings.cljc", &editable);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.actual.as_deref(), Some("Nil | String"));
    assert_eq!(
        error.contract.as_ref().unwrap().name,
        "java.lang.String/charAt"
    );
    assert_eq!(
        &editable[error.span.start.offset..error.span.end.offset],
        "name"
    );
}

#[test]
fn checks_member_arguments_and_marks_other_receivers_incomplete() {
    let source = r#"(defn indexed {:t/sig '[string? :-> char?]} [text] (.charAt text "0"))
         (defn unknown {:t/sig '[map? :-> number?]} [data] (.length data))"#;
    let report = Session::default().check("members.clj", source);
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    assert_eq!(report.diagnostics[0].expected.as_deref(), Some("Number"));
    assert_eq!(report.diagnostics[0].severity, "error");
    assert_eq!(report.diagnostics[1].severity, "warning");
    assert!(!report.complete);
}

#[test]
fn string_search_members_infer_boolean_and_numeric_results() {
    let source = r#"(ns urls (:require [clojure.spec.alpha :as s]))
        (defn http? {:t/sig '[(s/nilable string?) :-> boolean?]} [^String url]
          (if url (.startsWith url "http") false))
        (defn suffix? {:t/sig '[string? :-> boolean?]} [text]
          (. text (endsWith ".")))
        (defn first-dot {:t/sig '[string? :-> number?]} [text]
          (.indexOf ^java.lang.String text "."))"#;
    let report = Session::default().check("urls.cljc", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn string_search_members_reject_wrong_and_nil_arguments() {
    let source = r#"(ns urls)
        (defn wrong-prefix [] (.startsWith "http" :http))
        (defn nil-suffix [] (.endsWith "text" nil))
        (defn wrong-needle [] (.indexOf "prefix." ["."]))"#;
    let report = Session::default().check("urls.clj", source);
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == "error")
        .collect();
    assert_eq!(errors.len(), 3, "{report:#?}");
    for (error, (member, argument)) in errors.into_iter().zip([
        ("startsWith", ":http"),
        ("endsWith", "nil"),
        ("indexOf", r#"["."]"#),
    ]) {
        assert_eq!(error.code, "PNX2322");
        assert_eq!(
            error.contract.as_ref().unwrap().name,
            format!("java.lang.String/{member}")
        );
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            argument
        );
    }
}

#[test]
fn receiver_hints_resolve_members_without_proving_a_non_nil_string() {
    let source = r#"(ns urls (:require [clojure.spec.alpha :as s]))
        (defn hinted-param {:t/sig '[(s/nilable string?) :-> boolean?]} [^String url]
          (.startsWith url "http"))
        (defn hinted-use {:t/sig '[(s/nilable string?) :-> number?]} [text]
          (.indexOf ^java.lang.String text "."))
        (defn nil-receiver {:t/sig '[:-> boolean?]} [] (.endsWith nil "."))"#;
    let report = Session::default().check("urls.clj", source);
    assert_eq!(report.diagnostics.len(), 3, "{report:#?}");
    for (error, (member, actual, receiver)) in report.diagnostics.iter().zip([
        ("startsWith", "Nil | String", "url"),
        ("indexOf", "Nil | String", "text"),
        ("endsWith", "Nil", "nil"),
    ]) {
        assert_eq!(error.code, "PNX2322");
        assert_eq!(error.expected.as_deref(), Some("String"));
        assert_eq!(error.actual.as_deref(), Some(actual));
        assert_eq!(
            error.contract.as_ref().unwrap().name,
            format!("java.lang.String/{member}")
        );
        assert_eq!(
            &source[error.span.start.offset..error.span.end.offset],
            receiver
        );
    }
}

#[test]
fn string_search_reports_analysis_gaps_for_unknown_values_and_unmodeled_overloads() {
    let source = r#"(ns urls)
        (defn unknown-receiver {:t/sig '[map? :-> boolean?]} [m]
          (.startsWith (:url m) "http"))
        (defn hinted-unknown {:t/sig '[map? :-> boolean?]} [m]
          (let [^String url (:url m)] (.startsWith url "http")))
        (defn unknown-argument {:t/sig '[map? :-> boolean?]} [m]
          (.endsWith "text" (:suffix m)))
        (defn int-overload {:t/sig '[:-> number?]} [] (.indexOf "abc" 97))
        (defn offset-overload {:t/sig '[:-> boolean?]} [] (.startsWith "abc" "b" 1))
        (defn shadowed-hint {:t/sig '[string? :-> boolean?]} [^String url]
          (let [url {}] (.startsWith url "http")))
        (defn other-class {:t/sig '[map? :-> boolean?]} [m]
          (let [^java.nio.file.Path path (:path m)] (.startsWith path "dir")))"#;
    let report = Session::default().check("urls.clj", source);
    assert!(!report.complete, "{report:#?}");
    // The untyped map lookups also report their own missing type.
    let gaps: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code != "PNX9003")
        .collect();
    assert_eq!(gaps.len(), 7, "{report:#?}");
    for (gap, expression) in gaps.into_iter().zip([
        r#"(.startsWith (:url m) "http")"#,
        "url",
        "(:suffix m)",
        "97",
        r#"(.startsWith "abc" "b" 1)"#,
        r#"(.startsWith url "http")"#,
        r#"(.startsWith path "dir")"#,
    ]) {
        assert_eq!(gap.code, "PNX9001");
        assert_eq!(gap.severity, "warning");
        assert_eq!(
            &source[gap.span.start.offset..gap.span.end.offset],
            expression
        );
    }
}

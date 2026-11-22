use parenox::{DocumentHover, Report, Session};

fn check(body: &str) -> Report {
    let source =
        format!("(ns demo (:require [clojure.spec.alpha :as s] [clojure.string :as str]))\n{body}");
    Session::default().check("inline.clj", &source)
}

#[test]
fn a_conditional_operator_supplies_context_to_its_written_arguments() {
    let source = r#"(ns reports (:require [clojure.spec.alpha :as s]))
        (defn choose
          {:t/sig '[boolean? :-> (s/or :number int? :text string?)]} [numeric?]
          ((if numeric? #(inc %) str) (if numeric? 7 "label")))
        (defn number-label
          {:t/sig '[(s/fspec :args (s/cat :value number?) :ret string?) :-> string?]} [format]
          (format 7))
        (defn text-label
          {:t/sig '[(s/fspec :args (s/cat :value string?) :ret string?) :-> string?]} [format]
          (format "label"))
        (defn direct {:t/sig '[boolean? :-> string?]} [numeric?]
          ((if numeric? number-label text-label)
           (fn [message]
             (if numeric? (str (inc message)) (subs message 1)))))
        (defn before-arity {:t/sig '[:-> #{:argument}]} []
          (try
            ((if true inc str) (throw (ex-info "operand" {})) 2)
            (catch clojure.lang.ExceptionInfo _ :argument)))"#;
    let path = "reports.clj";
    let mut session = Session::default();
    let report = session.check(path, source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace("(str (inc message))", "(subs message 1)");
    let report = session.check(path, &edited);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        &edited[finding.span.start.offset..finding.span.end.offset],
        "message"
    );
    assert_eq!(
        finding.callback_contract.as_ref().unwrap().name,
        "reports/number-label"
    );
}

#[test]
fn raw_fn_uses_contextual_arguments_and_ordinary_body_maps() {
    for (expression, contract) in [
        (
            r#"((fn* ^{:pre [false]} [text] {:pre [false] :post [false]} text) "value")"#,
            "string?",
        ),
        (
            r#"((fn* other/self [again] (if again (self false) "ok")) true)"#,
            "string?",
        ),
        (
            r#"((fn* & [again] (if again (& false) "ok")) true)"#,
            "string?",
        ),
        (r#"((fn* .length [unused] (.length "abc")) nil)"#, "number?"),
        (
            r#"((fn* ([/ & more] (str / (first more)))) "a" "b")"#,
            "string?",
        ),
        (
            r#"(reduce (fn* ([acc item] (conj acc item))) [] ["one" "two"])"#,
            "(s/tuple string? string?)",
        ),
        (r#"(let [fn* (fn [] 19)] ((fn* [] "inner")))"#, "string?"),
        ("((fn* []))", "nil?"),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[:-> {contract}]}} [] {expression})"
        ));
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn raw_fn_checks_compiler_shape_even_without_invocation() {
    for (expression, code) in [
        ("((fn* [[value]] value) [1])", "PNX1001"),
        ("((fn* [value & [remaining]] value) 1)", "PNX1001"),
        ("(fn* [{:keys [value]}] value)", "PNX1001"),
        ("(fn* [bad.name] bad.name)", "PNX1001"),
        ("(fn* [] :first (missing-body-name))", "PNX2304"),
        ("(fn* [] {:pre [(subs nil 0)]} 1)", "PNX9001"),
        (
            "(try ((fn*)) (catch clojure.lang.ArityException _ :caught))",
            "PNX2554",
        ),
    ] {
        let report = check(&format!("(defn f {{:t/sig '[:-> t/any]}} [] {expression})"));
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{expression}: {report:#?}");
        };
        assert_eq!(finding.code, code, "{expression}: {report:#?}");
    }
}

#[test]
fn raw_fn_names_remain_values_when_the_same_head_is_jvm_syntax() {
    let direct = check(
        r#"(defn f {:t/sig '[:-> string?]} [] ((fn* String. [unused] (String. "abc")) nil))"#,
    );
    let [finding] = direct.diagnostics.as_slice() else {
        panic!("{direct:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
    assert!(finding.message.contains("JVM call"));

    let value = check(
        r#"(defn f {:t/sig '[:-> string?]} []
             ((fn* String. [flag] (if flag (apply String. [false]) "closure")) true))"#,
    );
    assert!(value.ok && value.complete, "{value:#?}");
}

#[test]
fn inline_parameters_capture_outer_locals_and_shadow_them_only_in_the_body() {
    for body in [
        r#"(defn f {:t/sig '[:-> number?]} []
             (let [delta 2] (:visits (update {:visits 3} :visits (fn [n] (+ n delta))))))"#,
        r#"(defn f {:t/sig '[:-> string?]} []
             (let [x "outer" _ ((fn [x] (inc x)) 2)] x))"#,
        r#"(defn f {:t/sig '[:-> number?]} []
             ((fn [{:keys [x] :or {x y}} y] x) {} 9))"#,
        r#"(defn f {:t/sig '[:-> number?]} []
             (let [scale 9] ((fn [{:keys [n] :or {n scale}}] n) {})))"#,
        r#"(defn f {:t/sig '[:-> number?]} [] ((fn [x x] x) "first" 2))"#,
        r#"(defn f {:t/sig '[:-> nil?]} [] ((fn [x]) 42))"#,
        r#"(defn f {:t/sig '[:-> number?]} [] ((fnil (fn [n] (inc n)) 0) nil))"#,
        r#"(defn f {:t/sig '[:-> number?]} [] (count ((fn [] {:pre [false]}))))"#,
        r#"(defn f {:t/sig '[:-> number?]} [] (let [f (fn [x] (inc x))] (f 1)))"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
}

#[test]
fn diagnoses_original_fn_body_and_unsupported_callable_forms() {
    let report = check(r#"(defn f {:t/sig '[:-> number?]} [] ((fn [x] (+ x x)) (inc "bad")))"#);
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");

    for (expr, code) in [
        ("((fn [x] x) 1 2)", "PNX2554"),
        ("((fn [:x] 1) 1)", "PNX1001"),
        ("((fn demo/walk [x] x) 1)", "PNX1001"),
        ("((fn [x] {:pre (false)} x) 1)", "PNX9001"),
        ("((fn ^{:post (false)} [x] x) 1)", "PNX9001"),
        ("(let [f (fn [x] x)] (identity f))", "PNX9001"),
    ] {
        let report = check(&format!("(defn f {{:t/sig '[:-> number?]}} [] {expr})"));
        assert_eq!(report.diagnostics[0].code, code, "{expr}: {report:#?}");
    }
}

#[test]
fn let_bound_callables_keep_their_captures_and_are_specialized_per_use() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
      (defn picked {:t/sig '[string? number? :-> string?]} [word n]
        (let [last-item (fn [_ item] item)
              reducer last-item]
          (str (subs (reduce reducer "" [word]) 0) (inc (reduce reducer 0 [n])))))
      (defn shadowed {:t/sig '[:-> string?]} []
        (let [prefix "first" step (fn [_ n] (subs prefix n)) prefix 7]
          (reduce step "" [0])))
      (defn guarded {:t/sig '[(s/nilable string?) :-> string?]} [prefix]
        (let [step (fn [_ n] (subs prefix n))]
          (if (some? prefix) (reduce step "" [0]) "")))"#;
    let mut session = Session::default();
    let report = session.check("saved.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    for (offset, ty) in [
        (
            source.find("item] item").unwrap() + "item] ".len(),
            "Number | String",
        ),
        (source.rfind("prefix n").unwrap(), "String"),
    ] {
        let Some(DocumentHover::Local(local)) = session.document_hover_at("saved.clj", offset)
        else {
            panic!("expected local type at byte {offset}");
        };
        assert_eq!(local.inferred_type, ty);
    }
}

#[test]
fn unused_let_bound_functions_compile_but_do_not_execute_and_expansion_is_bounded() {
    let report = check(
        r#"
        (defn ready {:t/sig '[:-> string?]} []
          (try
            (let [step (fn [_ _] (throw (ex-info "unreachable" {:stage :body})))]
              (reduce step "ready" []))
            (catch clojure.lang.ExceptionInfo _ 7)))
    "#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
    let report = check("(defn f {:t/sig '[:-> number?]} [] (let [bad (fn [x] (unbound x))] 0))");
    let [unbound] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(unbound.code, "PNX2304");

    let mut source =
        String::from("(ns demo) (defn run {:t/sig '[:-> number?]} [] (let [f0 (fn [x] (inc x))");
    for index in 1..16 {
        let previous = index - 1;
        source.push_str(&format!(
            " f{index} (fn [x] (+ (f{previous} x) (f{previous} x)))"
        ));
    }
    source.push_str("] (f15 0)))");
    let mut session = Session::default();
    let report = session.check("many.clj", &source);
    assert!(!report.complete, "{report:#?}");
    assert!(
        report.diagnostics.iter().any(|finding| {
            finding.code == "PNX9001" && finding.message.contains("Local fn analysis limit")
        }),
        "{report:#?}"
    );
    let offset = source.find("inc x").unwrap() + "inc ".len();
    assert!(session.document_hover_at("many.clj", offset).is_none());
}

#[test]
fn partial_preserves_target_identity_and_checks_bound_arguments_at_invocation() {
    for (expression, result) in [
        ("((partial (fn [prefix] prefix) \"saved\"))", "#{\"saved\"}"),
        ("((partial + 1 2 3 4) 5)", "number?"),
        (
            "((partial (partial (fn [a b] (str a b)) \"a\")) \"b\")",
            "string?",
        ),
        ("(reduce (partial 42 :saved) 0 [])", "number?"),
        (
            "(mapv (partial (fn [a b] (+ a b)) \"unused\") [])",
            "(s/coll-of number? :kind vector?)",
        ),
        (
            "(let [partial (fn [a b] (str a b))] (partial 1 2))",
            "string?",
        ),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[:-> {result}]}} [] {expression})"
        ));
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let rejected_identity = check("(defn f {:t/sig '[:-> number?]} [] (reduce (partial 42) 0 []))");
    assert_eq!(rejected_identity.diagnostics[0].code, "PNX2322");
    let missing_target = check(
        r#"
        (defn f {:t/sig '[:-> #{:caught}]} []
          (try ((partial) (inc "unused"))
               (catch clojure.lang.ArityException _ :caught)
               (catch IllegalArgumentException _ :wrong)))
    "#,
    );
    let [arity] = missing_target.diagnostics.as_slice() else {
        panic!("{missing_target:#?}")
    };
    assert_eq!(arity.code, "PNX2554");

    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn label {:t/sig '[string? number? :-> string?]} [prefix n] (str prefix n))
        (defn bad {:t/sig '[:-> (s/coll-of string? :kind vector?)]} []
          (mapv (partial label 9) [1]))"#;
    let report = Session::default().check("partial.clj", source);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(&source[error.span.start.offset..error.span.end.offset], "9");
    assert_eq!(error.contract.as_ref().unwrap().name, "demo/label");
}

#[test]
fn an_opaque_source_macro_can_shadow_partial_without_reusing_proofs() {
    let body = "(defn f {:t/sig '[:-> number?]} [] ((partial + 1) 2))";
    let path = "partial.clj";
    let clean = format!("(ns demo)\n{body}");
    let shadowed = format!("(ns demo)\n(defmacro partial [& _] :opaque)\n{body}");
    let qualified = shadowed.replace("((partial +", "((clojure.core/partial +");
    let mut session = Session::default();
    assert!(session.check(path, &clean).complete);
    let shadowed = session.check(path, &shadowed);
    assert!(!shadowed.complete, "{shadowed:#?}");
    assert!(
        shadowed
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX9001" && finding.message.contains("Macro partial")),
        "{shadowed:#?}"
    );
    assert_eq!(shadowed.stats.checked_functions, 1);
    let qualified = session.check(path, &qualified);
    let [gap] = qualified.diagnostics.as_slice() else {
        panic!("{qualified:#?}")
    };
    assert_eq!(gap.code, "PNX9001");
    assert!(gap.message.contains("defmacro"));
    let restored = session.check(path, &clean);
    assert!(restored.ok && restored.complete);
}

#[test]
fn partial_evaluates_its_target_and_captures_before_callback_input() {
    let report = check(
        r#"
        (defn f {:t/sig '[:-> #{:bound}]} []
          (try
            ((partial (fn [x y] (throw (ex-info "body" {:where :body})))
                      (throw (ex-info "bound" {:where :bound})))
             (throw (ex-info "caller" {:where :caller})))
            (catch clojure.lang.ExceptionInfo e (:where (ex-data e)))))
        (defn total {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [items]
          (apply (partial + 2 3) items))
    "#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn inline_overloads_check_the_selected_body_and_compile_all_clauses() {
    for (expr, result) in [
        (r#"((fn ([] "ready") ([n] (inc n))))"#, "string?"),
        (r#"((fn ([] "ready") ([n] (inc n))) 2)"#, "number?"),
        (
            "((fn ([x] x) ([x keep?] (if keep? (recur x false) x))) 2 true)",
            "number?",
        ),
        (
            "(mapv (fn ([] 0) ([n offset] (+ n offset))) [1] [2])",
            "(s/coll-of number? :kind vector?)",
        ),
        (
            "(mapv (fn ([] 0) ([n offset] (+ n offset))) [])",
            "(s/coll-of number? :kind vector?)",
        ),
    ] {
        let report = check(&format!("(defn f {{:t/sig '[:-> {result}]}} [] {expr})"));
        assert!(report.ok && report.complete, "{expr}: {report:#?}");
    }
    for (expr, code) in [
        ("((fn ([] 0) ([x] x)) 1 2)", "PNX2554"),
        ("((fn ([x] x) ([y] y)) 1)", "PNX2554"),
        ("((fn ([] 0) ([x] missing)))", "PNX2304"),
        ("((fn ([] 0) ([x] (recur))))", "PNX2554"),
    ] {
        let report = check(&format!("(defn f {{:t/sig '[:-> number?]}} [] {expr})"));
        assert_eq!(report.diagnostics.len(), 1, "{expr}: {report:#?}");
        assert_eq!(report.diagnostics[0].code, code, "{expr}: {report:#?}");
    }
}

#[test]
fn fnil_evaluates_its_callee_and_defaults_before_outer_arguments() {
    for (result, form) in [
        (
            "#{:default}",
            r#"(try
                  ((fnil identity (throw (ex-info "default" {:stage :default})))
                   (throw (ex-info "argument" {:stage :argument})))
                  (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))"#,
        ),
        (
            "#{:callee}",
            r#"(try
                  ((fnil (throw (ex-info "callee" {:stage :callee}))
                         (throw (ex-info "default" {:stage :default}))) nil)
                  (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))"#,
        ),
        ("number?", "(apply (fnil inc 0) [nil])"),
        ("number?", "(apply (fnil (fnil + 1 2) nil 5) [nil nil])"),
    ] {
        let source = format!("(ns demo) (defn run {{:t/sig '[:-> {result}]}} [] {form})");
        let report = Session::default().check("inline.clj", &source);
        assert!(report.ok && report.complete, "{form}: {report:#?}");
    }
}

use parenox::{Report, Session};

fn check(body: &str) -> Report {
    let source =
        format!("(ns demo (:require [clojure.spec.alpha :as s] [clojure.string :as str]))\n{body}");
    Session::default().check("inline.clj", &source)
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
        ("((fn walk [x] x) 1)", "PNX9001"),
        ("((fn [x] {:pre [false]} x) 1)", "PNX9001"),
        ("((fn ^{:post [false]} [x] x) 1)", "PNX9001"),
        ("(let [f (fn [x] x)] (f 1))", "PNX9001"),
    ] {
        let report = check(&format!("(defn f {{:t/sig '[:-> number?]}} [] {expr})"));
        assert_eq!(report.diagnostics[0].code, code, "{expr}: {report:#?}");
    }
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

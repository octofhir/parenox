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
        ("((fn ([x] x) ([x y] y)) 1)", "PNX9001"),
        ("((fn [x & rest] x) 1)", "PNX9001"),
        ("((fn [x] {:pre [false]} x) 1)", "PNX9001"),
        ("((fn ^{:post [false]} [x] x) 1)", "PNX9001"),
        ("(let [f (fn [x] x)] (f 1))", "PNX9001"),
    ] {
        let report = check(&format!("(defn f {{:t/sig '[:-> number?]}} [] {expr})"));
        assert_eq!(report.diagnostics[0].code, code, "{expr}: {report:#?}");
    }
}

use parenox::{Report, Session};

fn check(expression: &str, contract: &str) -> Report {
    let source = format!(
        "(ns demo (:require [clojure.string :as str]))\n(defn f {{:t/sig '[:-> {contract}]}} [] {expression})"
    );
    Session::default().check("loops.clj", &source)
}

#[test]
fn checks_loop_accumulators_and_types_of_later_iterations() {
    let source = include_str!("../examples/loops.clj");
    let report = Session::default().check("loops.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let source = include_str!("../examples/loops_bad.clj");
    let report = Session::default().check("loops_bad.clj", source);
    assert!(report.complete && !report.ok, "{report:#?}");
    let codes: Vec<_> = report.diagnostics.iter().map(|d| d.code.as_str()).collect();
    assert_eq!(codes, ["PNX2322", "PNX2554", "PNX1001", "PNX2554"]);
    let mismatch = &report.diagnostics[0];
    assert_eq!(mismatch.actual.as_deref(), Some("String"));
    assert_eq!(mismatch.expected.as_deref(), Some("Number"));
    assert_eq!(
        &source[mismatch.span.start.offset..mismatch.span.end.offset],
        "value"
    );
    assert!(mismatch.notes[0].contains("recur at 6:7"));
    assert!(
        report.diagnostics[1]
            .message
            .contains("to loop expects 2 arguments")
    );
    assert!(
        report.diagnostics[3]
            .message
            .contains("to fn expects 1 arguments")
    );
}

#[test]
fn checks_tail_position_after_core_macro_expansion() {
    for expression in [
        "(loop [i 0] (if (= i 0) i (do i (let [n (inc i)] (recur n)))))",
        "(loop [i 0] (or true (recur i)))",
        "(loop [i 0] (and false (recur i)))",
        "(loop [i 0] (when-let [value nil] (recur i)))",
        "(loop [i 0] (if-some [value nil] (recur i) i))",
        "(loop [i 0] (when false (recur i)))",
        "(loop [i 0] (if true i (-> i recur)))",
        "(loop [i 0] (if true i (some-> i recur)))",
        "(loop [i 0] (cond-> i false recur))",
        "(loop [i 0] (if true i (as-> i n (recur n))))",
        "(loop [i 0] (if true i (-> (recur i))))",
        "(loop [i 0] (let [recur 3] (if true i (recur i))))",
        "(loop [i 0] (let [loop #{0}] (loop i)))",
        "(loop* [i 0] i)",
        "(loop [] (recur))",
        "(loop [])",
    ] {
        let report = check(
            expression,
            "(clojure.spec.alpha/nilable (clojure.spec.alpha/or :number number? :bool boolean?))",
        );
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }

    for expression in [
        "(loop [i 0] (if (recur i) i 0))",
        "(loop [i 0] (do (recur i) i))",
        "(loop [i 0] (let [n (recur i)] n))",
        "(loop [i 0] (or (recur i) true))",
        "(loop [i 0] (when false (do (recur i) i)))",
        "(loop [i 0] (if-some [n (recur i)] n i))",
        "(loop [i 0] (some-> (recur i)))",
        "(loop [i 0] (cond-> (recur i)))",
        "(loop [i 0] (as-> (recur i) x))",
        "(loop [i 0] (cond-> i false recur true inc))",
        "(loop [i 0] (as-> i x (recur x) (inc x)))",
        "(loop [i 0] (recur (recur i)))",
        "(loop [i 0] [(recur i)])",
    ] {
        let report = check(expression, "number?");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == "PNX1001"
                    && d.message == "recur is only allowed in tail position"),
            "{expression}: {report:#?}"
        );
    }
}

#[test]
fn each_nested_loop_or_function_sets_its_own_recur_target() {
    let report = check("(loop [a 1 b 2] (loop [inner a] (recur a b)))", "number?");
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert!(
        report.diagnostics[0]
            .message
            .contains("to loop expects 1 arguments")
    );

    for expression in [
        "(loop [a 1 b 2] (+ (loop [first? true] (if first? (recur false) a)) b))",
        "((fn [a b] (if a (recur false (inc b)) b)) true 1)",
        "(loop [a 1 b 2] ((fn [x] (recur x)) a))",
        "(loop [[x] [1] y x] (+ x y))",
        "(let [y 1] (loop [{:keys [x] :or {x y}} {} y 2] x))",
        "(loop [a 0] (if true a (recur \"unreachable\")))",
    ] {
        let report = check(expression, "number?");
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }

    let report = Session::default().check(
        "fn.clj",
        r#"
        (ns demo)
        (defn- iterate-label [first? result]
          (if first? (recur false "result") result))
        (defn public {:t/sig '[boolean? nil? :-> string?]} [first? value]
          (if first? (recur false nil) (iterate-label true value)))
    "#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn growing_loop_types_report_an_analysis_limit() {
    let report = check("(loop [value 1] (recur [value value]))", "number?");
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
    assert!(
        report.diagnostics[0]
            .message
            .contains("type inference limit")
    );
}

#[test]
fn one_bad_destructuring_site_has_one_diagnostic() {
    let report = check("(loop [{:keys! [n]} {}] n)", "number?");
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX2741");
}

use parenox::{Report, Session};

fn check(source: &str) -> Report {
    Session::default().check("private_data.clj", source)
}

#[test]
fn unused_private_functions_can_build_data_from_plain_parameters() {
    let report = check(
        r#"
(ns demo.private-data (:require [clojure.spec.alpha :as s]))
(defn- constant [] {:status :ready :nothing nil :flag false :marker ::s/invalid})
(defn- pass [x] x)
(defn ^:private documented "Build a field." [x] {:value x})
(defn- wider [a b c d e] [e d c b a])
(defn- nested [left right]
  {:payload {:right right :repeated [left right left]}
   :trace [nil {:left left :flag false}]
   :status :ready})
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
    assert!(report.diagnostics.is_empty(), "{report:#?}");
}

#[test]
fn dead_calls_do_not_make_a_proven_data_body_unchecked() {
    let report = check(
        r#"
(ns demo.private-data)
(defn- packet [x] {:value x})
(defn- box [x] [x])
(defn unreachable {:t/sig '[:-> #{:skipped}]} []
  (if true :skipped (packet :input)))
(defn throwing-argument {:t/sig '[:-> #{:caught}]} []
  (try (box (throw (ex-info "argument" {})))
       (catch ExceptionInfo _ :caught)))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn direct_calls_preserve_nil_false_and_the_spec_marker_as_data() {
    let source = r#"
(ns demo.private-data (:require [clojure.spec.alpha :as s]))
(defn- packet [value missing flag]
  {:payload {:mark value :flag flag} :trace [missing flag value] :tag :ready})
(defn exact
  {:t/sig '[:-> {:payload {:mark #{::s/invalid} :flag #{false}}
                 :trace (s/tuple nil? #{false} #{::s/invalid})
                 :tag #{:ready}}]} []
  (packet ::s/invalid nil false))
(defn another
  {:t/sig '[:-> {:payload {:mark #{false} :flag #{:flag}}
                 :trace (s/tuple #{::s/invalid} #{:flag} #{false})
                 :tag #{:ready}}]} []
  (packet false ::s/invalid :flag))
"#;
    let report = check(source);
    assert!(report.ok && report.complete, "{report:#?}");

    let wrong = check(&source.replace(
        "(packet ::s/invalid nil false)",
        "(packet :ready nil false)",
    ));
    let [finding] = wrong.diagnostics.as_slice() else {
        panic!("{wrong:#?}");
    };
    assert!(wrong.complete, "{wrong:#?}");
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert!(
        finding.message.contains("demo.private-data/exact"),
        "{finding:#?}"
    );
}

#[test]
fn unused_private_bodies_outside_the_data_subset_keep_a_contract_gap() {
    for definition in [
        "(defn- helper [x] (inc x))",
        "(defn- helper [x] [{:value (inc x)}])",
        "(defn- helper [{:keys [x]}] [x])",
        "(defn- helper [x & xs] [x xs])",
        "(defn- helper ([x] x) ([x y] [x y]))",
        "(defn- helper [^String x] [x])",
        "(defn- helper [x] ^{:example true} [x])",
        "(defn- helper {:note (throw (ex-info \"attribute\" {}))} [] 1)",
        "(defn- ^{:note (inc 41)} helper [] 1)",
        "(defn- helper [x] {x :payload})",
        "(defn- helper [x] #{x :payload})",
        "(def outside :payload) (defn- helper [] [outside])",
    ] {
        let report = check(&format!("(ns demo.private-data)\n{definition}"));
        assert!(!report.ok && !report.complete, "{definition}: {report:#?}");
        assert!(
            report.diagnostics.iter().any(|finding| {
                finding.code == "PNX9002" && finding.message.contains("demo.private-data/helper")
            }),
            "{definition}: {report:#?}"
        );
    }
}

#[test]
fn public_and_first_class_functions_still_need_contracts() {
    for body in [
        "(defn packet [x] {:value x})",
        r#"(defn- packet [x] {:value x})
           (defn uses-value {:t/sig '[:-> any?]} []
             (let [f packet] (f :ready)))"#,
    ] {
        let report = check(&format!("(ns demo.private-data)\n{body}"));
        assert!(!report.ok && !report.complete, "{body}: {report:#?}");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|finding| finding.code == "PNX9002"),
            "{body}: {report:#?}"
        );
    }
}

#[test]
fn warm_edits_recheck_the_private_body_without_rechecking_an_independent_root() {
    let valid = r#"
(ns demo.private-data)
(defn independent {:t/sig '[:-> #{:stable}]} [] :stable)
(defn- packet [x] {:value [x]})
"#;
    let unchecked = valid.replace("[x]}", "[(identity x)]}");
    let mut session = Session::default();
    let path = "private_data.clj";

    let initial = session.check(path, valid);
    assert!(initial.ok && initial.complete, "{initial:#?}");
    assert_eq!(initial.stats.checked_functions, 1, "{initial:#?}");
    let warm = session.check(path, valid);
    assert_eq!(warm.stats.checked_functions, 0, "{warm:#?}");
    assert_eq!(warm.stats.reused_functions, 1, "{warm:#?}");

    for _ in 0..2 {
        let changed = session.check(path, &unchecked);
        assert!(!changed.ok && !changed.complete, "{changed:#?}");
        let [finding] = changed.diagnostics.as_slice() else {
            panic!("{changed:#?}");
        };
        assert_eq!(finding.code, "PNX9002", "{finding:#?}");
        assert!(finding.message.contains("demo.private-data/packet"));
        assert!(
            unchecked[finding.span.start.offset..finding.span.end.offset].contains("(identity x)"),
            "{finding:#?}"
        );
        assert_eq!(changed.stats.checked_functions, 0, "{changed:#?}");
        assert_eq!(changed.stats.reused_functions, 1, "{changed:#?}");
    }

    let restored = session.check(path, valid);
    assert!(restored.ok && restored.complete, "{restored:#?}");
    assert_eq!(restored.stats.checked_functions, 0, "{restored:#?}");
    assert_eq!(restored.stats.reused_functions, 1, "{restored:#?}");
}

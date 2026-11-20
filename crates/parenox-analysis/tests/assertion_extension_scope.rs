use parenox_analysis::{ProjectReport, Report, Session, Source};

const APP: Source = Source {
    path: "app_test.clj",
    source: r#"
(ns demo.checked-test (:require [clojure.test :as t] [clojure.string :as str]))
(t/deftest checked (t/is (= "" (str/trim 42))))
"#,
};

#[test]
fn an_unrelated_assertion_extension_can_appear_and_disappear_in_a_warm_snapshot() {
    let mut session = Session::default();
    let standard = session.check_project(&[APP], None);
    assert_standard_dispatch(&standard);

    for source in [
        r#"
(ns demo.extension (:require [clojure.test :as t]))
(def dispatch (resolve 't/assert-expr))
(.addMethod ^clojure.lang.MultiFn @dispatch '= (fn [_ _] true))
"#,
        r#"
(ns demo.extension (:require [clojure.test :as t]))
(def dispatch (resolve (first (keys '#::t{assert-expr true}))))
(.addMethod ^clojure.lang.MultiFn @dispatch '= (fn [_ _] true))
"#,
        r#"
(ns demo.extension (:require [clojure.test :refer [assert-expr]
                              :rename {assert-expr custom-dispatch}]))
(defmethod custom-dispatch '= [_ _] true)
"#,
    ] {
        let extension = Source {
            path: "extension.clj",
            source,
        };
        let blocked = session.check_project(&[APP, extension], None);
        assert_opaque_dispatch(&blocked);
        let warm_blocked = session.check_project(&[APP, extension], None);
        assert_opaque_dispatch(&warm_blocked);

        let restored = session.check_project(&[APP], None);
        assert_standard_dispatch(&restored);
        assert_eq!(
            app_report(&restored).diagnostics[0].id,
            app_report(&standard).diagnostics[0].id
        );
    }
}

#[test]
fn inert_assertion_extension_spellings_preserve_standard_dispatch() {
    let mut session = Session::default();
    for (path, source) in [
        (
            "provider.clj",
            r#"(ns demo.provider) (def data "assert-expr")"#,
        ),
        (
            "provider.clj",
            "(ns demo.provider)\n;; assert-expr\n(def data :safe)",
        ),
        (
            "provider.clj",
            "(ns demo.provider (:require [clojure.test :as t]))\n#_(resolve 't/assert-expr)\n(def data :safe)",
        ),
        (
            "provider.cljc",
            "(ns demo.provider (:require [clojure.test :as t]))\n#?(:clj (def data :safe) :cljs (resolve 't/assert-expr))",
        ),
        (
            "provider.clj",
            "(ns demo.provider (:require [unrelated :as-alias u]))\n(def data '#::u{assert-expr true})",
        ),
    ] {
        let report = session.check_project(&[APP, Source { path, source }], None);
        assert_standard_dispatch(&report);
    }
}

fn app_report(report: &ProjectReport) -> &Report {
    report
        .files
        .iter()
        .find(|file| file.path == APP.path)
        .unwrap()
}

fn assert_standard_dispatch(project: &ProjectReport) {
    let report = app_report(project);
    assert!(report.complete && !report.ok, "{project:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{project:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        &APP.source[finding.span.start.offset..finding.span.end.offset],
        "42"
    );
}

fn assert_opaque_dispatch(project: &ProjectReport) {
    let report = app_report(project);
    assert!(!report.complete, "{project:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{project:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
    assert!(finding.message.contains("t/deftest"), "{finding:#?}");
}

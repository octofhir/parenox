use parenox::{Report, Session};

fn check(source: &str) -> Report {
    Session::default().check("helper_effects.clj", source)
}

#[test]
fn local_inferred_calls_report_exact_effects_and_stop_on_failed_arguments() {
    let report = check(include_str!("../examples/helper_effects.clj"));
    assert!(report.ok && report.complete, "{report:#?}");
    let report = check(include_str!("../examples/helper_effects_bad.clj"));
    assert!(!report.ok && report.complete, "{report:#?}");
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.actual.as_deref(), Some(":missing"));
}

#[test]
fn specializations_merge_their_exceptions_into_each_handler_scope() {
    let report = check(
        r#"(defn- fail [stage]
              (throw (ex-info "request failed" {:stage stage})))
           (defn f {:t/sig '[:-> #{:missing}]} []
             (try (fail :missing)
                  (catch ExceptionInfo e (:stage (ex-data e))))
             (try (fail :missing)
                  (catch ExceptionInfo e (:stage (ex-data e)))))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn a_specialization_first_checked_without_a_handler_can_be_tracked_later() {
    let report = check(
        r#"(defn- require-ready [ready?]
              (if ready? :ready (throw (ex-info "not ready" {:stage :missing}))))
           (defn f {:t/sig '[boolean? :-> #{:ready :missing}]} [ready?]
             (require-ready ready?)
             (try
               (require-ready ready?)
               (catch ExceptionInfo e (:stage (ex-data e)))))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

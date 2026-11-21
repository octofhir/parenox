use parenox::{Session, output};

#[test]
fn suppression_matches_only_its_rule_inside_the_next_form() {
    let source = r#"(defn f {:t/sig '[boolean? :-> number?]} [check-legacy?]
      ;; parenox-ignore PNX2322: the legacy input is intentional
      (if check-legacy? (do (inc "hidden") (missing)) 0)
      (inc "visible"))"#;
    let report = Session::default().check("app.clj", source);
    assert!(!report.ok && !report.complete);
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    let [suppressed] = report.suppressed.as_slice() else {
        panic!("{report:#?}");
    };
    let diag = &suppressed.diagnostic;
    assert_eq!(diag.code, "PNX2322");
    assert_eq!(diag.span.start.line, 3);
    assert_eq!(suppressed.directive.start.line, 2);
    assert_eq!(suppressed.reason, "the legacy input is intentional");
    assert!(diag.id.is_some());
    assert_eq!(report.diagnostics[0].code, "PNX2304");
    assert_eq!(report.diagnostics[1].code, "PNX2322");
    assert_eq!(report.diagnostics[1].span.start.line, 4);
}

#[test]
fn removing_a_comment_restores_the_cached_finding_and_its_id() {
    let source = r#"
        ;; parenox-ignore PNX2322: legacy boundary
        (defn label {:t/sig '[:-> string?]} [] 42)"#;
    let mut session = Session::default();
    let first = session.check("app.clj", source);
    assert!(first.ok && !first.complete, "{first:#?}");
    assert!(first.diagnostics.is_empty());
    assert_eq!(first.suppressed.len(), 1);
    let id = &first.suppressed[0].diagnostic.id;
    let json = serde_json::to_value(&first).unwrap();
    assert_eq!(
        json["suppressed"][0]["diagnostic"]["id"].as_str(),
        id.as_deref()
    );
    for rendered in [output::plain(&first), output::pretty(&first, source, false)] {
        assert!(rendered.contains("suppressed PNX2322"), "{rendered}");
        assert!(rendered.contains("legacy boundary"), "{rendered}");
    }
    let warm = session.check("app.clj", source);
    assert_eq!(warm.stats.reused_functions, 1);
    assert_eq!(&warm.suppressed[0].diagnostic.id, id);

    let edited = source.replace("parenox-ignore", "parenox-notice");
    let restored = session.check("app.clj", &edited);
    assert_eq!(restored.stats.reused_functions, 1);
    assert!(!restored.ok && restored.complete);
    assert!(restored.suppressed.is_empty());
    assert_eq!(&restored.diagnostics[0].id, id);
}

#[test]
fn rejects_invalid_directives_and_reports_unused_ones_after_a_clean_check() {
    for directive in [
        "PNX2322",
        "PNX2322:   ",
        "PNX9001: not suppressible",
        "PNX9999: unknown code",
    ] {
        let source = format!("; parenox-ignore {directive}\n(ns app)");
        let report = Session::default().check("app.clj", &source);
        assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
        assert_eq!(report.diagnostics[0].code, "PNX9006");
        assert!(report.suppressed.is_empty());
    }
    let end = "(defn f {:t/sig '[:-> number?]} [] 1\n; parenox-ignore PNX2322: no next form\n)";
    let report = Session::default().check("app.clj", end);
    assert_eq!(report.diagnostics[0].code, "PNX9006");

    let unused = "(defn f {:t/sig '[:-> number?]} []\n; parenox-ignore PNX2322: old issue\n1)";
    let report = Session::default().check("app.clj", unused);
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9007");
    assert!(report.diagnostics[0].id.is_some());

    let unfinished = unused.replace("\n1)", "\n#vendor/opaque {})");
    let report = Session::default().check("app.clj", &unfinished);
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
}

#[test]
fn ignores_comments_in_discarded_quoted_and_inactive_forms() {
    let source = r#"(ns app)
      #_(discard
         ; parenox-ignore PNX9999: ignored in discarded data
         1)
      (defn f {:t/sig '[:-> number?]} []
        (quote
          ; parenox-ignore PNX9999: ignored in quoted data
          1)
        #?(:cljs (do
                    ; parenox-ignore PNX9999: ignored in an inactive branch
                    0)
           :clj (inc
                   ; parenox-ignore PNX2322: supported branch only
                   "text")))"#;
    let report = Session::default().check("app.cljc", source);
    assert!(report.ok && !report.complete, "{report:#?}");
    assert_eq!(report.suppressed.len(), 1);
    assert_eq!(report.suppressed[0].reason, "supported branch only");
}

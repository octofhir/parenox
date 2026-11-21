use parenox_analysis::{DocumentHover, ExternPathStep, Session};

#[test]
fn fixed_doc_rows_retain_their_positions_at_the_use_site() {
    let source = r#"
(ns completion-rows (:require [clojure.spec.alpha :as s]))

(defn overview {:t/sig '[#{"overview"} :-> #{"overview"}]} [row] row)

;; clojure-lsp.feature.completion lowers documentation with this mapv step.
(defn render-doc-rows {:t/sig '[:-> #{"overview"}]} []
  (let [documentation ["overview" {:value "example"}]
        rows (->> documentation
                  (mapv #(or (:value %) %)))]
    (overview (nth rows 0))))
"#;
    let mut session = Session::default();
    let report = session.check("completion_rows.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let binding = source.find("rows (->>").unwrap();
    let Some(DocumentHover::Local(rows)) =
        session.document_hover_at("completion_rows.clj", binding)
    else {
        panic!("expected hover on rendered rows");
    };
    assert_eq!(rows.inferred_type, r#"Tuple<"overview", "example">"#);

    let changed = source.replace("(nth rows 0)", "(nth rows 1)");
    let report = session.check("completion_rows.clj", &changed);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert!(finding.message.contains(r#"got "example""#), "{finding:#?}");
    assert_eq!(
        &changed[finding.span.start.offset..finding.span.end.offset],
        "(nth rows 1)"
    );
    assert!(session.check("completion_rows.clj", source).ok);
}

#[test]
fn fixed_mapv_visits_only_runtime_positions_and_blames_the_literal_element() {
    let source = r#"
(ns mapv-positions (:require [clojure.spec.alpha :as s]))
(defn zipped {:t/sig '[:-> (s/tuple (s/tuple #{:left} #{:right}))]} []
  (mapv (fn [left right] [left right]) [:left :unvisited] [:right]))
(defn nested {:t/sig '[:-> (s/tuple #{false} #{true})]} []
  (mapv vector? (mapv identity [{:value :map} [:vector]])))
(defn stopped {:t/sig '[:-> #{:caught}]} []
  (try (mapv (fn [item]
               (if (= item :stop)
                 (throw (ex-info "stop" {}))
                 (let [visited item] visited)))
             [:first :stop :unvisited])
       (catch clojure.lang.ExceptionInfo _ :caught)))
(defn empty-result {:t/sig '[:-> (s/coll-of any? :kind vector?)]} []
  (mapv (fn [uncalled _] uncalled) [:first] []))
(defn structural {:t/sig '[(s/tuple #{:shape}) :-> any?]} [input]
  (let [unchecked-order (mapv identity input)] unchecked-order))
(defn render {:t/sig '[string? :-> string?]} [x] x)
"#;
    let mut session = Session::default();
    let report = session.check("mapv_positions.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let visited = source.find("visited item").unwrap();
    let Some(DocumentHover::Local(item)) = session.document_hover_at("mapv_positions.clj", visited)
    else {
        panic!("expected hover on visited element");
    };
    assert_eq!(item.inferred_type, ":first");
    let uncalled = source.find("uncalled _").unwrap();
    assert!(
        session
            .document_hover_at("mapv_positions.clj", uncalled)
            .is_none()
    );
    let structural = source.find("unchecked-order (mapv").unwrap();
    let Some(DocumentHover::Local(rows)) =
        session.document_hover_at("mapv_positions.clj", structural)
    else {
        panic!("expected hover on structurally typed input's result");
    };
    assert_eq!(rows.inferred_type, "Vector<:shape>");

    let changed =
        format!("{source}\n(defn wrong {{:t/sig '[:-> any?]}} [] (mapv render [\"ok\" 0x7]))");
    let report = session.check("mapv_positions.clj", &changed);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        &changed[finding.span.start.offset..finding.span.end.offset],
        "0x7"
    );
}

#[test]
fn filterv_preserves_the_positions_retained_by_malli_instrumentation() {
    let source = r#"
(ns malli-instrumented)
(defn installed {:t/sig '[:-> qualified-symbol?]} []
  (let [actions (filterv some? ['sample.instrument/run nil 'sample.instrument/stop])]
    (nth actions 1)))
"#;
    let mut session = Session::default();
    let report = session.check("malli_instrumented.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let binding = source.find("actions (filterv").unwrap();
    let Some(DocumentHover::Local(actions)) =
        session.document_hover_at("malli_instrumented.clj", binding)
    else {
        panic!("expected hover on the retained actions");
    };
    assert_eq!(
        actions.inferred_type,
        "Tuple<'sample.instrument/run, 'sample.instrument/stop>"
    );

    let broken = source.replace("'sample.instrument/stop", "nil");
    let report = session.check("malli_instrumented.clj", &broken);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2801");
    let span = finding.span.start.offset..finding.span.end.offset;
    assert_eq!(&broken[span], "1");
    assert!(session.check("malli_instrumented.clj", source).ok);
}

#[test]
fn projected_literal_fields_keep_their_callback_argument_position() {
    let source = r#"
(ns positions)
(defn line-delta {:t/sig '[{:line number?} {:line number?} :-> number?]} [left right]
  (- (:line right) (:line left)))
(defn check-positions {:t/sig '[:-> any?]} []
  (let [original [{:line 0}]]
    (mapv line-delta original [{:line "0"}])))
"#;
    let report = Session::default().check("positions.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    let span = finding.span.start.offset..finding.span.end.offset;
    assert_eq!(&source[span], r#""0""#);
    assert_eq!(
        finding.mismatch_path,
        [
            ExternPathStep::Argument { index: 2 },
            ExternPathStep::Field {
                key: ":line".into()
            },
        ]
    );
    assert_eq!(
        finding.contract.as_ref().unwrap().name,
        "positions/line-delta"
    );
}

use parenox_analysis::{DocumentHover, Session};

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

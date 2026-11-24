use parenox::{Report, Session, Source};

fn check(source: &str) -> Report {
    Session::default().check("demo.clj", source)
}

#[test]
fn input_stream_read_supports_direct_overloads_and_callback_uses() {
    let report = check(
        r#"
        (ns demo (:require [clojure.spec.alpha :as s])
                 (:import [java.io InputStream]))
        (defn single {:t/sig '[t/any :-> int?]} [in] (InputStream/.read in))
        (defn buffered {:t/sig '[t/any bytes? :-> int?]} [in buffer]
          (InputStream/.read in buffer))
        (defn sliced {:t/sig '[t/any bytes? :-> int?]} [in buffer]
          (java.io.InputStream/.read in buffer 0.0 4))
        (defn via-map {:t/sig '[t/any :-> (s/coll-of int? :kind seq?)]} [in]
          (map InputStream/.read [in]))
        "#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn input_stream_read_diagnoses_wrong_arity_receiver_and_buffer() {
    let report = check(
        r#"
        (ns demo (:import [java.io InputStream]))
        (defn arity {:t/sig '[t/any :-> t/any]} [in]
          (InputStream/.read in (byte-array 8) 0))
        (defn receiver {:t/sig '[:-> t/any]} []
          (InputStream/.read "not a stream"))
        (defn buffer {:t/sig '[t/any :-> t/any]} [in]
          (InputStream/.read in "not a byte array"))
        "#,
    );
    for (code, line) in [("PNX2554", 4), ("PNX2322", 6), ("PNX2322", 8)] {
        assert!(
            report
                .diagnostics
                .iter()
                .any(|finding| { finding.code == code && finding.span.start.line == line }),
            "{report:#?}"
        );
    }
}

#[test]
fn exception_member_diagnostics_respect_qualified_and_reflective_dispatch() {
    let report = check(
        r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn qualified {:t/sig '[t/any :-> t/any]} [e]
          (Exception/.getException e))
        (defn known {:t/sig '[:-> t/any]} []
          (try (throw (ex-info "failed" {}))
               (catch Exception e (.getException e))))
        (defn unknown {:t/sig '[(s/fspec :args (s/cat) :ret t/any) :-> t/any]} [call]
          (try (call)
               (catch Exception e (. e getException))))
        "#,
    );
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|finding| finding.code == "PNX2339")
            .count(),
        2,
        "{report:#?}"
    );
    assert!(
        report.diagnostics.iter().any(|finding| {
            finding.code == "PNX9001" && finding.message.contains("runtime subclass")
        }),
        "{report:#?}"
    );
}

#[test]
fn iterable_iteration_preserves_jvm_hints_after_map_narrowing() {
    let report = check(
        r#"
        (ns demo (:require [clojure.spec.alpha :as s])
                 (:import [java.util Iterator]))
        (defn tagged-local {:t/sig '[t/any :-> (s/nilable boolean?)]}
          [^Iterable input]
          (when (map? input)
            (let [items (.iterator input)]
              (.hasNext items))))
        (defn tagged-receiver {:t/sig '[t/any :-> (s/nilable Iterator)]} [input]
          (when (map? input) (. ^Iterable input (iterator))))
        (defn unhinted {:t/sig '[t/any :-> (s/nilable java.util.Iterator)]} [input]
          (when (map? input) (.iterator input)))
        (defn tagged-vector {:t/sig '[t/any :-> (s/nilable Iterator)]} [^Iterable input]
          (when (vector? input) (.iterator input)))
        (defn read-next {:t/sig '[Iterator :-> t/any]} [items] (.next items))
        (defn via-map {:t/sig '[:-> (s/coll-of Iterator :kind seq?)]} []
          (map Iterable/.iterator [{:a 1}]))
        (defn qualified {:t/sig '[:-> boolean?]} []
          (Iterator/.hasNext (java.lang.Iterable/.iterator {:a 1})))
        "#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn iteration_checks_qualified_arity_and_proven_incompatible_receivers() {
    let report = check(
        r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn wrong-arity {:t/sig '[:-> t/any]} []
          (Iterable/.iterator {} :extra))
        (defn wrong-class {:t/sig '[:-> t/any]} []
          (let [^Iterable input "text"] (.iterator input)))
        (defn nullable {:t/sig '[(s/nilable java.util.Iterator) :-> t/any]} [input]
          (.hasNext input))
        (defn wrong-iterator {:t/sig '[:-> t/any]} []
          (java.util.Iterator/.hasNext {}))
        "#,
    );
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| (finding.code.as_str(), finding.span.start.line))
        .collect();
    assert_eq!(
        errors,
        [
            ("PNX2554", 4),
            ("PNX2322", 6),
            ("PNX2322", 8),
            ("PNX2322", 10)
        ],
        "{report:#?}"
    );
}

#[test]
fn iterator_types_are_preserved_in_source_free_contracts() {
    let provider = Source {
        path: "maps.clj",
        source: r#"
        (ns maps (:import [java.util Iterator]))
        (defn entries {:t/sig '[map? :-> Iterator]} [data] (.iterator data))
        "#,
    };
    let export = Session::default().export_externs(&[provider], &[]);
    let edn = export.edn.unwrap_or_else(|| panic!("{:#?}", export.report));
    let client = Source {
        path: "client.clj",
        source: r#"
        (ns client (:require [maps :as m]))
        (defn nonempty? {:t/sig '[map? :-> boolean?]} [data]
          (.hasNext (m/entries data)))
        "#,
    };
    let report = Session::default().check_project(
        &[client],
        Some(Source {
            path: "maps.edn",
            source: &edn,
        }),
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

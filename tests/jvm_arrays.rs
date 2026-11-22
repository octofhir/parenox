use parenox::Session;

#[test]
fn byte_arrays_supply_type_evidence_independent_of_local_hints() {
    let source = r#"
(defn by-alias {:t/sig '[:-> int?]} []
  (let [^bytes buffer (byte-array 2)] (alength buffer)))
(defn by-descriptor {:t/sig '[bytes? :-> int?]} [value]
  (let [^"[B" buffer value] (alength buffer)))
(defn by-array-class {:t/sig '[bytes? :-> int?]} [value]
  (let [^byte/1 buffer value] (alength buffer)))
(defn by-guard {:t/sig '[t/any :-> int?]} [value]
  (let [^bytes buffer value]
    (if (bytes? buffer) (alength buffer) 0)))
(defn dynamic-call {:t/sig '[:-> int?]} []
  (apply alength [(byte-array [7 258])]))
(defn empty-array {:t/sig '[:-> boolean?]} []
  (bytes? (byte-array nil)))
"#;
    let report = Session::default().check("arrays.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn array_length_accepts_the_stock_array_aliases_and_descriptors() {
    for (hint, initializer) in [
        ("booleans", "(boolean-array [true false])"),
        ("bytes", "(byte-array [1 2])"),
        ("chars", r"(char-array [\a \b])"),
        ("shorts", "(short-array [1 2])"),
        ("ints", "(int-array [1 2])"),
        ("longs", "(long-array [1 2])"),
        ("floats", "(float-array [1.0 2.0])"),
        ("doubles", "(double-array [1.0 2.0])"),
        ("objects", "(object-array [:a :b])"),
        ("String/1", "(object-array [:a :b])"),
        (r#""[Ljava.lang.String;""#, "(object-array [:a :b])"),
        (r#""[[B""#, "(object-array [:a :b])"),
    ] {
        let source = format!(
            "(defn length {{:t/sig '[:-> int?]}} [] (let [^{hint} buffer {initializer}] (alength buffer)))"
        );
        let report = Session::default().check("arrays.clj", &source);
        assert!(report.ok && report.complete, "{source}: {report:#?}");
    }
}

#[test]
fn hinted_array_operations_distinguish_nil_wrong_class_and_unproved_values() {
    let source = r#"
(defn wrong-class {:t/sig '[bytes? :-> #{:cast}]} [value]
  (let [^ints original value buffer original]
    (try (alength buffer)
      (catch IllegalArgumentException _ (inc "wrong handler"))
      (catch ClassCastException _ :cast))))
(defn absent {:t/sig '[nil? :-> #{:null}]} [value]
  (let [^bytes buffer value]
    (try (alength buffer)
      (catch ClassCastException _ (inc "wrong handler"))
      (catch NullPointerException _ :null))))
(defn bad-dispatch {:t/sig '[string? :-> #{:dispatch}]} [text]
  (try (alength text)
    (catch ClassCastException _ (inc "wrong handler"))
    (catch IllegalArgumentException _ :dispatch)))
(defn open {:t/sig '[t/any :-> int?]} [value]
  (let [^bytes buffer value] (alength buffer)))
"#;
    let report = Session::default().check("arrays.clj", source);
    assert!(!report.complete, "{report:#?}");
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                finding.expected.as_deref(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX2322", Some("int[]"), "buffer"),
            ("PNX2322", Some("byte[]"), "buffer"),
            ("PNX2322", Some("JVM array"), "text"),
            ("PNX9001", None, "buffer"),
        ]
    );
}

#[test]
fn array_hints_follow_jvm_class_selection_in_do_and_if() {
    let source = r#"
(defn outer-hints {:t/sig '[boolean? :-> int?]} [flag]
  (let [data (int-array 2)]
    (alength ^bytes (do (alength ^bytes (if flag data data)) data))))
(defn tail-hint {:t/sig '[:-> #{:cast}]} []
  (let [data (int-array 2)
        buffer (do :ignored ^bytes data)]
    (try (alength buffer)
      (catch ClassCastException _ :cast))))
(defn common-class {:t/sig '[boolean? :-> #{:cast :null}]} [flag]
  (let [data (int-array 2)
        buffer (if flag ^bytes data nil)]
    (try (alength buffer)
      (catch ClassCastException _ :cast)
      (catch NullPointerException _ :null))))
"#;
    let report = Session::default().check("arrays.clj", source);
    assert!(report.complete, "{report:#?}");
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                finding.expected.as_deref(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX2322", Some("byte[]"), "buffer"),
            ("PNX2322", Some("byte[]"), "buffer"),
        ]
    );
}

#[test]
fn byte_array_sizes_follow_number_int_value() {
    let source = r#"
(defn wrapped {:t/sig '[:-> int?]} []
  (alength (byte-array 4294967298)))
(defn negative {:t/sig '[:-> #{:negative}]} []
  (try (alength (byte-array -1))
    (catch ClassCastException _ (inc "wrong handler"))
    (catch NegativeArraySizeException _ :negative)))
"#;
    let report = Session::default().check("arrays.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2801");
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "-1"
    );
}

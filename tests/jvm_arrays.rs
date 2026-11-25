use parenox::Session;

#[test]
fn boolean_and_char_array_setters_preserve_the_original_argument() {
    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(defn nil-flag {:t/sig '[:-> nil?]} []
  (aset-boolean (boolean-array 1) 0 nil))
(defn truthy {:t/sig '[:-> #{"kept"}]} []
  (first (aset-boolean (boolean-array 1) 0 ["kept"])))
(defn dynamic-flag {:t/sig '[t/any :-> t/any]} [flag]
  (aset-boolean (boolean-array 1) 0 flag))
(defn char-number {:t/sig '[:-> (s/and int? #{65})]} []
  (aset-char (char-array 1) 0 65))
(defn widened-char {:t/sig '[:-> #{\x}]} []
  (apply aset-char [(int-array 1) 0 \x]))
(defn dynamic-char {:t/sig '[char? :-> char?]} [value]
  (try (aset-char (char-array 1) 0 value)
       (catch IllegalArgumentException _ :wrong)))
"#;
    let report = Session::default().check("arrays.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    for (body, exception, code) in [
        (
            "(aset-boolean (int-array 1) 0 true)",
            "IllegalArgumentException",
            "PNX2322",
        ),
        (
            "(aset-char (short-array 1) 0 65)",
            "IllegalArgumentException",
            "PNX2322",
        ),
        (
            "(aset-char (char-array 1) 0 65536)",
            "IllegalArgumentException",
            "PNX2801",
        ),
        (
            "(aset-char (char-array 1) 0 -1)",
            "IllegalArgumentException",
            "PNX2801",
        ),
        (
            "(aset-char (char-array 1) 0 nil)",
            "NullPointerException",
            "PNX2322",
        ),
        (
            r"(aset-boolean (boolean-array 1) \u0000 true)",
            "ClassCastException",
            "PNX2322",
        ),
        (
            "(aset-boolean (boolean-array 0) 0 nil)",
            "ArrayIndexOutOfBoundsException",
            "PNX2801",
        ),
    ] {
        let source = format!(
            "(defn f {{:t/sig '[:-> #{{:caught}}]}} []
               (try (do {body} :returned) (catch {exception} _ :caught)))"
        );
        let report = Session::default().check("arrays.clj", &source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert!(report.complete, "{report:#?}");
        assert_eq!(finding.code, code, "{report:#?}");
    }
}

#[test]
fn numeric_array_setters_widen_stores_and_return_original_values() {
    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(defn byte-write {:t/sig '[:-> (s/and int? #{42})]} []
  (aset-byte (double-array 1) 0 42))
(defn short-write {:t/sig '[:-> #{\x}]} []
  (aset-short (int-array 1) 0 \x))
(defn int-write {:t/sig '[:-> (s/and integer? #{9})]} []
  (apply aset-int [(long-array 1) 0 9N]))
(defn long-write {:t/sig '[:-> (s/and int? #{13})]} []
  (aset-long (float-array 1) 0 13))
(defn float-write {:t/sig '[:-> (s/and int? #{4})]} []
  (aset-float (double-array 1) 0 4))
(defn double-write {:t/sig '[:-> number?]} []
  (aset-double (double-array 1) 0 4.5))
"#;
    let report = Session::default().check("arrays.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    for (body, exception, code) in [
        (
            "(aset-long (int-array 1) 0 2)",
            "IllegalArgumentException",
            "PNX2322",
        ),
        (
            "(aset-byte (byte-array 1) 0 128)",
            "IllegalArgumentException",
            "PNX2801",
        ),
        (
            "(aset-int (int-array 1) 0 4294967296N)",
            "ArithmeticException",
            "PNX2801",
        ),
        (
            "(aset-long (long-array 1) 0 9223372036854775808N)",
            "IllegalArgumentException",
            "PNX2801",
        ),
        (
            "(aset-double (double-array 1) 0 nil)",
            "NullPointerException",
            "PNX2322",
        ),
        (
            r"(aset-float (float-array 1) 0 \x)",
            "ClassCastException",
            "PNX2322",
        ),
        (
            r"(aset-short (short-array 1) \u0000 1)",
            "ClassCastException",
            "PNX2322",
        ),
        (
            "(aset-int (int-array 2) 2 1)",
            "ArrayIndexOutOfBoundsException",
            "PNX2801",
        ),
    ] {
        let source = format!(
            "(defn f {{:t/sig '[:-> #{{:caught}}]}} []
               (try (do {body} :returned) (catch {exception} _ :caught)))"
        );
        let report = Session::default().check("arrays.clj", &source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert!(report.complete, "{report:#?}");
        assert_eq!(finding.code, code, "{report:#?}");
    }

    for source in [
        "(defn f {:t/sig '[:-> t/any]} [] (aset-int (object-array [(int-array 1)]) 0 0 1))",
        r#"(ns demo (:require [clojure.spec.alpha :as s]))
            (defn write {:t/sig '[bytes? (s/coll-of int? :kind vector? :min-count 2) :-> t/any]}
              [buffer indices-and-value] (apply aset-byte buffer indices-and-value))"#,
    ] {
        let report = Session::default().check("arrays.clj", source);
        assert!(!report.complete, "{report:#?}");
        assert_eq!(report.diagnostics[0].code, "PNX9001");
    }
}

#[test]
fn aset_checks_reference_boolean_and_character_array_writes() {
    let source = r#"
(ns demo)
(defn store-reference {:t/sig '[:-> #{"saved"}]} []
  (first (aset (object-array 1) 0 ["saved"])))
(defn store-flag {:t/sig '[:-> #{false}]} []
  (aset (boolean-array 1) \u0000 false))
(defn store-char {:t/sig '[:-> #{\x}]} []
  (apply aset [(char-array 1) 0 \x]))
(defn store-nil {:t/sig '[:-> nil?]} []
  (aset (object-array 1) 0 nil))
(defn read-after-write {:t/sig '[:-> boolean?]} []
  (let [flags (boolean-array 1)]
    (aset flags 0 true)
    (nth flags 0 :missing)))
"#;
    let report = Session::default().check("arrays.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    for (body, exception, code) in [
        (
            "(aset (object-array 1) 1 :item)",
            "ArrayIndexOutOfBoundsException",
            "PNX2801",
        ),
        (
            "(aset (object-array 1) 4294967296N :item)",
            "ArithmeticException",
            "PNX2801",
        ),
        (
            "(aset (boolean-array 1) 0 :bad)",
            "IllegalArgumentException",
            "PNX2322",
        ),
        (
            "(apply aset [(char-array 1) 0 97])",
            "IllegalArgumentException",
            "PNX2322",
        ),
    ] {
        let source = format!(
            "(defn f {{:t/sig '[:-> #{{:caught}}]}} []
               (try (do {body} :returned) (catch {exception} _ :caught)))"
        );
        let report = Session::default().check("arrays.clj", &source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert!(report.complete, "{report:#?}");
        assert_eq!(finding.code, code, "{report:#?}");
    }

    for body in [
        "(aset (byte-array 1) 0 3)",
        "(aset (object-array [(boolean-array 1)]) 0 0 false)",
    ] {
        let source = format!("(defn f {{:t/sig '[:-> t/any]}} [] {body})");
        let report = Session::default().check("arrays.clj", &source);
        assert!(!report.complete, "{report:#?}");
        assert_eq!(report.diagnostics[0].code, "PNX9001");
    }
}

#[test]
fn aclone_preserves_array_class_and_length_for_direct_and_apply_calls() {
    let source = r#"
(ns demo)
(defn clone-buffer {:t/sig '[bytes? :-> bytes?]} [^bytes buffer] (aclone buffer))
(defn clone-size {:t/sig '[:-> #{true}]} []
  (= 3 (alength (apply aclone [(object-array [:a :b :c])]))))
(defn clone-read {:t/sig '[:-> boolean?]} []
  (nth (aclone (boolean-array 1)) 0 :missing))
(defn clone-empty {:t/sig '[:-> nil?]} []
  (seq (aclone (char-array 0))))
"#;
    let report = Session::default().check("arrays.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    for (body, exception) in [
        (r#"(aclone "not an array")"#, "IllegalArgumentException"),
        ("(apply aclone [nil])", "NullPointerException"),
        (
            "(let [^ints a (byte-array 1)] (aclone a))",
            "ClassCastException",
        ),
    ] {
        let source = format!(
            "(defn f {{:t/sig '[:-> #{{:caught}}]}} []
               (try (do {body} :returned) (catch {exception} _ :caught)))"
        );
        let report = Session::default().check("arrays.clj", &source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert!(report.complete, "{report:#?}");
        assert_eq!(finding.code, "PNX2322");
    }
}

#[test]
fn aget_respects_a_loop_guard_against_the_array_length() {
    let source = r#"
(defn scan {:t/sig '[:-> #{:done}]} []
  (let [marker (byte-array [1 2 3 4])
        length (alength marker)]
    (loop [j 0]
      (if (>= j length)
        :done
        (do (aget marker j) (recur (inc j)))))))
"#;
    let report = Session::default().check("arrays.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let off_by_one = source.replace("(>= j length)", "(> j length)");
    let report = Session::default().check("arrays.clj", &off_by_one);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2801");
    assert!(finding.message.contains("Index 4"), "{finding:#?}");
}

#[test]
fn aget_checks_boxed_reads_hints_and_indices() {
    let source = r#"
(ns demo)
(defn byte-at {:t/sig '[bytes? int? :-> int?]} [^bytes buffer i] (aget buffer i))
(defn flag {:t/sig '[:-> boolean?]} [] (aget (boolean-array 1) 0))
(defn via-apply {:t/sig '[:-> char?]} [] (apply aget [(char-array [\x]) 0]))
(defn fractional {:t/sig '[:-> int?]} [] (aget (byte-array 2) 0.5))
(defn character-index {:t/sig '[:-> int?]} [] (aget (byte-array 2) \u0000))
(defn in-bounds {:t/sig '[:-> int?]} []
  (try (aget (byte-array 2) 1)
       (catch IndexOutOfBoundsException _ :wrong)))
"#;
    let report = Session::default().check("arrays.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    for (body, exception, code) in [
        (
            "(aget (byte-array 2) 2)",
            "ArrayIndexOutOfBoundsException",
            "PNX2801",
        ),
        (
            "(aget (byte-array 2) 4294967296N)",
            "ArithmeticException",
            "PNX2801",
        ),
        (
            "(aget (byte-array 2) :bad)",
            "ClassCastException",
            "PNX2322",
        ),
        (
            "(let [^ints a (byte-array 1)] (aget a 0))",
            "ClassCastException",
            "PNX2322",
        ),
        (
            r"(apply aget [(byte-array 2) \u0000])",
            "ClassCastException",
            "PNX2322",
        ),
    ] {
        let source = format!(
            "(defn f {{:t/sig '[:-> #{{:caught}}]}} []
               (try (do {body} :returned) (catch {exception} _ :caught)))"
        );
        let report = Session::default().check("arrays.clj", &source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert!(report.complete, "{report:#?}");
        assert_eq!(finding.code, code, "{report:#?}");
    }

    for source in [
        "(defn f {:t/sig '[:-> t/any]} [] (aget (object-array [(byte-array 2)]) 0 1))",
        r#"(ns demo (:require [clojure.spec.alpha :as s]))
            (defn f {:t/sig '[bytes? (s/coll-of int? :kind vector? :min-count 1) :-> t/any]}
              [buffer indices] (apply aget buffer indices))"#,
    ] {
        let report = Session::default().check("arrays.clj", source);
        assert!(
            !report.complete
                && report
                    .diagnostics
                    .iter()
                    .any(|finding| finding.code == "PNX9001"),
            "{report:#?}"
        );
    }
}

#[test]
fn array_contains_checks_numeric_indices_and_refines_guarded_reads() {
    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(defn present {:t/sig '[:-> #{true}]} []
  (contains? (byte-array 2) 4294967296N))
(defn absent {:t/sig '[:-> #{false}]} []
  (contains? (byte-array 2) 2))
(defn negative {:t/sig '[:-> #{false}]} []
  (contains? (byte-array 2) -1))
(defn empty-array {:t/sig '[int? :-> #{false}]} [i]
  (contains? (byte-array 0) i))
(defn float-index {:t/sig '[:-> boolean?]} []
  (contains? (byte-array 2) 0.5))
(defn guarded {:t/sig '[bytes? int? :-> int?]} [buffer i]
  (if (contains? buffer i)
    (get buffer i :missing)
    0))
(defn absent-default {:t/sig '[bytes? int? :-> #{:present :missing}]} [buffer i]
  (if (contains? buffer i) :present (get buffer i :missing)))
"#;
    let report = Session::default().check("arrays.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let unstable = source.replace("bytes? int? :-> int?", "bytes? number? :-> int?");
    let report = Session::default().check("arrays.clj", &unstable);
    assert!(!report.ok && report.complete, "{report:#?}");

    for key in ["nil", ":not-an-index"] {
        let source = format!(
            "(defn f {{:t/sig '[:-> #{{:caught}}]}} []
              (try (do (contains? (byte-array 1) {key}) :returned)
                   (catch IllegalArgumentException _ :caught)))"
        );
        let report = Session::default().check("arrays.clj", &source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert!(report.complete, "{report:#?}");
        assert_eq!(finding.code, "PNX2322");
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            key
        );
    }
}

#[test]
fn get_on_arrays_uses_number_int_value_and_preserves_missing_defaults() {
    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(defn read-at {:t/sig '[:-> int?]} []
  (get (byte-array 2) 1 :missing))
(defn wrapped-index {:t/sig '[:-> boolean?]} []
  (try (get (boolean-array 1) 4294967296N :missing)
       (catch ArithmeticException _ :wrong)))
(defn negative-index {:t/sig '[:-> #{:missing}]} []
  (get (byte-array 2) 4294967295 :missing))
(defn missing-key {:t/sig '[:-> #{"missing"}]} []
  (first (get (byte-array 2) :not-an-index ["missing"])))
(defn fallback-with-bounds {:t/sig '[:-> #{:missing}]} []
  (get (byte-array 2) 2 :missing))
(defn dynamic-index {:t/sig '[bytes? int? :-> (s/nilable int?)]} [buffer index]
  (get buffer index))
(defn empty-array {:t/sig '[number? :-> #{:missing}]} [index]
  (get (byte-array 0) index :missing))
(defn required-index {:t/sig '[:-> int?]} []
  (try (req! (byte-array 1) 4294967296)
       (catch IllegalArgumentException _ :wrong)))
"#;
    let report = Session::default().check("arrays.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let source = "(defn read-array {:t/sig '[:-> int?]} [] (req! (byte-array 2) 2))";
    let report = Session::default().check("arrays.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2741");
}

#[test]
fn array_reads_use_known_lengths_without_assuming_element_values() {
    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(defn narrowed-size {:t/sig '[:-> #{true}]} []
  (= 2 (alength (byte-array 4294967298))))
(defn wrapped-size {:t/sig '[:-> #{true}]} []
  (let [[array] [(object-array [:a :b :c])]] (= 3 (alength array))))
(defn joined-size {:t/sig '[boolean? :-> #{true}]} [flag]
  (= 2 (count (if flag (byte-array 2) (byte-array [3 4])))))
(defn empty-seq {:t/sig '[:-> nil?]} [] (seq (int-array nil)))
(defn empty-array {:t/sig '[:-> #{true}]} [] (empty? (int-array 0)))
(defn empty-buffer {:t/sig '[:-> nil?]} [] (not-empty (byte-array 0)))
(defn preserved-array {:t/sig '[:-> bytes?]} [] (not-empty (byte-array 1)))
(defn first-element {:t/sig '[:-> int?]} [] (first (byte-array 2)))
(defn tail-count {:t/sig '[:-> #{true}]} []
  (= 999999 (count (next (int-array 1000000)))))
(defn in-bounds {:t/sig '[:-> boolean?]} []
  (try (nth (boolean-array 1) 0)
       (catch ArrayIndexOutOfBoundsException _ :wrong)))
(defn out-of-bounds {:t/sig '[:-> #{"missing"}]} []
  (first (nth (byte-array 2) 2 ["missing"])))
"#;
    let report = Session::default().check("arrays.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let source = "(defn read-array {:t/sig '[:-> int?]} [] (nth (byte-array 2) 2))";
    let report = Session::default().check("arrays.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2801");
    assert!(finding.message.contains("length 2"), "{finding:#?}");
}

#[test]
fn nth_on_arrays_checks_element_type_fallbacks_and_jvm_index_bounds() {
    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(defn byte-at {:t/sig '[bytes? int? :-> int?]} [buffer i]
  (nth buffer i))
(defn flag-at {:t/sig '[:-> boolean?]} []
  (nth (boolean-array [false]) 0 nil))
(defn ref-at {:t/sig '[:-> t/any]} []
  (nth (object-array [nil]) 0 :missing))
(defn negative-fallback {:t/sig '[:-> #{"fallback"}]} []
  (first (nth (byte-array 2) -1 ["fallback"])))
(defn maximum-fallback {:t/sig '[:-> #{:missing}]} []
  (nth (byte-array 2) 2147483647 :missing))
(defn maybe-bounds {:t/sig '[bytes? :-> (s/or :value int? :bounds #{:bounds})]} [buffer]
  (try (nth buffer 0)
       (catch ArrayIndexOutOfBoundsException _ :bounds)
       (catch ArithmeticException _ :wrong)))
"#;
    let report = Session::default().check("arrays.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    for (index, fallback, exception) in [
        ("-1", "", "ArrayIndexOutOfBoundsException"),
        ("2147483647", "", "IndexOutOfBoundsException"),
        ("2147483648", " :missing", "ArithmeticException"),
        ("4294967296N", " :missing", "ArithmeticException"),
    ] {
        let source = format!(
            "(defn read-array {{:t/sig '[:-> #{{:caught}}]}} []
               (try (do (nth (byte-array 2) {index}{fallback}) :returned)
                    (catch {exception} _ :caught)
                    (catch IllegalArgumentException _ :wrong)))"
        );
        let report = Session::default().check("arrays.clj", &source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert!(report.complete, "{report:#?}");
        assert_eq!(finding.code, "PNX2801");
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            index
        );
    }
}

#[test]
fn array_sequence_reads_preserve_boxed_types_and_possible_emptiness() {
    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(defn bytes-seq {:t/sig '[bytes? :-> (s/nilable (s/coll-of int? :kind seq? :min-count 1))]}
  [buffer] (seq buffer))
(defn head {:t/sig '[:-> (s/nilable boolean?)]} []
  (first (boolean-array [false true])))
(defn tail {:t/sig '[:-> (s/nilable (s/coll-of char? :kind seq? :min-count 1))]} []
  (next (char-array [\a \b])))
(defn remaining {:t/sig '[:-> (s/coll-of number? :kind seq?)]} []
  (rest (double-array 0)))
(defn size {:t/sig '[bytes? :-> int?]} [buffer] (count buffer))
(defn empty-buffer {:t/sig '[bytes? :-> boolean?]} [buffer] (empty? buffer))
(defn nonempty-buffer {:t/sig '[bytes? :-> (s/nilable bytes?)]} [buffer] (not-empty buffer))
"#;
    let report = Session::default().check("arrays.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(defn nonempty {:t/sig '[bytes? :-> (s/coll-of int? :kind seq? :min-count 1)]}
  [buffer] (seq buffer))
(defn wrong-element {:t/sig '[:-> (s/nilable keyword?)]} []
  (first (boolean-array 2)))
"#;
    let report = Session::default().check("arrays.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
}

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

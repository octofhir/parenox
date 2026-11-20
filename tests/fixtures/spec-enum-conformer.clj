(ns lsp4clj.coercer (:require [clojure.spec.alpha :as s]))

;; lsp4clj 2.0.1, coercer.clj lines 9–12.
(def file-change-type-enum {1 :created 2 :changed 3 :deleted})
(s/def :file-event/type (s/and int?
                               file-change-type-enum
                               (s/conformer file-change-type-enum)))

;; JVM-only assertions: clojure -M tests/fixtures/spec-enum-conformer.clj

(doseq [[input expected] [[1 :created] [2 :changed] [3 :deleted]
                          [(int 1) :created] [(short 1) :created]
                          [0x1 :created] [2/2 :created]]]
  (assert (s/valid? :file-event/type input))
  (assert (= expected (s/conform :file-event/type input))))

(doseq [input [4 1N (biginteger 1) 1.0 "1" nil 2147483648]]
  (assert (not (s/valid? :file-event/type input)))
  (assert (= ::s/invalid (s/conform :file-event/type input))))

(def mixed {1 false 2 nil 3 ::s/invalid 4 :ok 5N :big-key})
(s/def ::mixed (s/and int? mixed (s/conformer mixed)))
(doseq [input [1 2 3 5N 6]]
  (assert (not (s/valid? ::mixed input)))
  (assert (= ::s/invalid (s/conform ::mixed input))))
(assert (= :ok (s/conform ::mixed 4)))
(assert (= :big-key (s/conform ::mixed 5)))

;; The standalone contract uses the same table in both stages.
(s/def ::inline
  (s/and int? {1 :created 2 :changed 3 :deleted}
         (s/conformer {1 :created 2 :changed 3 :deleted})))

(defn assert-result [spec input expected]
  (assert (identical? expected (s/conform spec input)))
  (assert (= (not= ::s/invalid expected) (s/valid? spec input))))

(doseq [spec [:file-event/type ::inline]
        box [byte short int long]
        [input expected] [[1 :created] [2 :changed] [3 :deleted]]]
  (assert-result spec (box input) expected))

(doseq [spec [:file-event/type ::inline]
        input [nil false true ::s/invalid :created "1" 1N (biginteger "1")
               (float 1) 1.0 1.0M 3/2 0 -1 4 2147483648 Long/MIN_VALUE Long/MAX_VALUE
               -9223372036854775809N 9223372036854775808N]]
  (assert-result spec input ::s/invalid))

;; Fixed JVM integers match integral numeric keys by value across boxing classes.
(s/def ::inline-boundaries
  (s/and int? {-9223372036854775808N :min 9223372036854775807N :max 2147483648N :wide
               4N :big-key 5.0 :floating 6M :decimal 9223372036854775808N :too-wide}
         (s/conformer
          {-9223372036854775808N :min 9223372036854775807N :max 2147483648N :wide
           4N :big-key 5.0 :floating 6M :decimal 9223372036854775808N :too-wide})))
(doseq [[input expected] [[Long/MIN_VALUE :min] [Long/MAX_VALUE :max]
                          [2147483648 :wide] [(byte 4) :big-key]
                          [4N ::s/invalid] [(biginteger "4") ::s/invalid]
                          [5 ::s/invalid] [6 ::s/invalid] [9223372036854775808N ::s/invalid]]]
  (assert-result ::inline-boundaries input expected))
(doseq [literal ["{1 :first 1N :second}" "{1 :first 0x1 :second}" "{1 :first 2/2 :second}"]]
  (assert (try (read-string literal)
               false
               (catch IllegalArgumentException _ true))))

(let [custom (proxy [Number] []
               (equals [_] (throw (IllegalStateException. "unexpected lookup"))))]
  (assert (number? custom))
  (assert (not (int? custom)))
  (assert-result :file-event/type custom ::s/invalid)
  (assert-result ::inline custom ::s/invalid))

;; The two stages capture the evaluated tables when the spec is registered.
(let [original file-change-type-enum]
  (with-redefs [file-change-type-enum {1 :temporary}]
    (assert-result :file-event/type 1 :created)
    (assert-result :file-event/type 2 :changed)
    (s/def ::captured-temporary
      (s/and int? file-change-type-enum (s/conformer file-change-type-enum))))
  (assert-result ::captured-temporary 1 :temporary)
  (assert-result ::captured-temporary 2 ::s/invalid)
  (try
    (alter-var-root #'file-change-type-enum (constantly {1 :replacement 4 :new}))
    (assert-result :file-event/type 1 :created)
    (assert-result :file-event/type 4 ::s/invalid)
    (s/def ::captured-root
      (s/and int? file-change-type-enum (s/conformer file-change-type-enum)))
    (finally (alter-var-root #'file-change-type-enum (constantly original))))
  (assert-result ::captured-root 1 :replacement)
  (assert-result ::captured-root 4 :new)
  (assert-result :file-event/type 1 :created))

;; A predicate table and a different conformer table need not agree on truthiness.
(s/def ::distinct-stages
  (s/and int? {1 :yes 2 :yes 3 :yes}
         (s/conformer {1 nil 2 false 3 ::s/invalid})))
(assert-result ::distinct-stages 1 nil)
(assert-result ::distinct-stages 2 false)
(assert-result ::distinct-stages 3 ::s/invalid)

(prn {:clojure (clojure-version) :spec-enum-conformer :ok})

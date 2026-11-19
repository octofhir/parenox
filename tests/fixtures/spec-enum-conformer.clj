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

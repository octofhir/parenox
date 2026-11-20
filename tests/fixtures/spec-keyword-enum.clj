(ns demo.keyword-enum (:require [clojure.spec.alpha :as s]))

;; The completion kind and text format use the lookup pattern from lsp4clj 2.0.1.
(def completion-item-kinds {:text 1 :method 2 :typeparameter 25})
(s/def :completion-item/kind
  (s/and keyword? completion-item-kinds (s/conformer completion-item-kinds)))

(def insert-text-formats {:plaintext 1 :snippet 2})
(s/def :completion-item/insert-text-format
  (s/and keyword? insert-text-formats (s/conformer insert-text-formats)))

(s/def ::label string?)
(s/def ::completion-item
  (s/keys :req-un [::label] :opt-un [:completion-item/kind :completion-item/insert-text-format]))
(s/def ::completion-items (s/coll-of ::completion-item))

(def edge-codes {:zero 0 :false false :nil nil :marker ::s/invalid ::s/invalid 17 :foreign/known 7})
(s/def ::edge
  (s/and clojure.core/keyword? demo.keyword-enum/edge-codes (s/conformer demo.keyword-enum/edge-codes)))
(s/def ::edge-alias ::edge)

;; JVM-only assertions. Run with clojure -M tests/fixtures/spec-keyword-enum.clj

(doseq [[spec input expected] [[:completion-item/kind :text 1]
                               [:completion-item/kind :method 2]
                               [:completion-item/kind :typeparameter 25]
                               [:completion-item/insert-text-format :plaintext 1]
                               [:completion-item/insert-text-format :snippet 2]
                               [::edge :zero 0]
                               [::edge-alias :foreign/known 7]]]
  (let [actual (s/conform spec input)]
    (assert (s/valid? spec input))
    (assert (int? actual))
    (assert (= expected actual))))

(doseq [input [:false :nil :marker ::s/invalid :unknown :known :foreign/unknown 1 "zero" false nil]]
  (assert (not (s/valid? ::edge input)))
  (assert (= ::s/invalid (s/conform ::edge input))))

(let [item {:label "complete" :kind :method :insert-text-format :snippet}
      unchanged {:label "complete"}
      expected {:label "complete" :kind 2 :insert-text-format 2}
      entry (clojure.lang.MapEntry/create item item)
      untouched (clojure.lang.MapEntry/create unchanged unchanged)
      rebuilt (s/conform ::completion-items entry)]
  (assert (= expected (s/conform ::completion-item item)))
  (assert (= [expected expected] rebuilt))
  (assert (not (map-entry? rebuilt)))
  (assert (identical? untouched (s/conform ::completion-items untouched))))

(prn :keyword-enum-ok (clojure-version))

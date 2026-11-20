(ns demo.inverse-enum
  (:require [clojure.set :as set] [clojure.spec.alpha :as s]))

;; The tables and input specs follow lsp4clj 2.0.1.
(def kinds
  {:text 1 :method 2 :function 3 :constructor 4 :field 5 :variable 6 :class 7 :interface 8 :module 9
   :property 10 :unit 11 :value 12 :enum 13 :keyword 14 :snippet 15 :color 16 :file 17 :reference 18
   :folder 19 :enummember 20 :constant 21 :struct 22 :event 23 :operator 24 :typeparameter 25})
(def kinds-by-code (set/map-invert kinds))
(s/def :input.completion-item/kind
  (s/and integer? kinds-by-code (s/conformer kinds-by-code)))

(def formats {:plaintext 1 :snippet 2})
(def formats-by-code (clojure.set/map-invert formats))
(s/def :input.completion-item/insert-text-format
  (s/and clojure.core/integer? formats-by-code (s/conformer formats-by-code)))
(s/def ::format :input.completion-item/insert-text-format)
(s/def ::boxed-code (s/and integer? (s/conformer int)))

(s/def ::input-item (s/keys :opt-un [:input.completion-item/kind :input.completion-item/insert-text-format]))
(s/def ::input-items (s/coll-of ::input-item))

(def edge {:zero 0 :operator/negative -2 ::s/invalid 9})
(def edge-by-code (set/map-invert edge))
(s/def ::edge (s/and integer? edge-by-code (s/conformer edge-by-code)))

(def fixed-formats {1 :plaintext 2 :snippet})
(s/def ::fixed-format (s/and int? fixed-formats (s/conformer fixed-formats)))

;; JVM-only assertions. Run with clojure -M tests/fixtures/spec-inverse-enum.clj

(doseq [input [(Long/valueOf "1") (Integer/valueOf "1") (Short/valueOf "1") (Byte/valueOf "1")
               1N (biginteger 1) 2/2 9223372036854775808/9223372036854775808 0x1 01 2r1]]
  (assert (integer? input))
  (assert (s/valid? :input.completion-item/kind input))
  (assert (identical? :text (s/conform :input.completion-item/kind input)))
  (assert (identical? :plaintext (s/conform ::format input))))

(doseq [[key input] kinds]
  (assert (identical? key (s/conform :input.completion-item/kind input))))
(let [boxed (s/conform ::boxed-code 1N)]
  (assert (= Integer (type boxed)))
  (assert (= :plaintext (s/conform ::format boxed))))

(doseq [input [nil false "1" :text 1.0 1M 3/2 0 -1 26 26N 2147483648 9223372036854775808N ::s/invalid]]
  (assert (not (s/valid? :input.completion-item/kind input)))
  (assert (= ::s/invalid (s/conform :input.completion-item/kind input))))
(doseq [[input result] [[0 :zero] [0N :zero] [-2N :operator/negative]]]
  (assert (s/valid? ::edge input))
  (assert (identical? result (s/conform ::edge input))))
(assert (not (s/valid? ::edge 9)))
(assert (= ::s/invalid (s/conform ::edge 9)))
(assert (= ::s/invalid (get edge-by-code 9)))
(assert (s/valid? ::fixed-format 1))
(assert (not (s/valid? ::fixed-format 1N)))
(assert (= ::s/invalid (s/conform ::fixed-format 1N)))

(s/def ::explicit-input (s/and integer? #{1 2}))
(doseq [input [1 1N 2/2 2N]]
  (assert (s/valid? ::explicit-input input))
  (assert (identical? input (s/conform ::explicit-input input))))
(doseq [input [1.0 1M 3/2 3N]]
  (assert (not (s/valid? ::explicit-input input))))

(let [item {:kind 1N :insert-text-format 4/2 :extra true}
      expected {:kind :text :insert-text-format :snippet :extra true}
      entry (clojure.lang.MapEntry/create item item)
      result (s/conform ::input-items entry)
      untouched (clojure.lang.MapEntry/create {:extra true} {})]
  (assert (= expected (s/conform ::input-item item)))
  (assert (= [expected expected] result))
  (assert (not (map-entry? result)))
  (assert (identical? untouched (s/conform ::input-items untouched)))
  (assert (= ::s/invalid (s/conform ::input-item {:kind 1.0}))))

(prn :inverse-enum-spec-ok (clojure-version))

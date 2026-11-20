(ns checked-map-predicate
  (:import [clojure.lang IPersistentMap]))

;; JVM reference: clojure -M tests/fixtures/checked-map-predicate.clj

(defrecord Reading [value])

(doseq [value [{} {:value 1} (hash-map :value 1) (sorted-map :value 1) (->Reading 1)]]
  (assert (map? value)))

(let [entry (first {:value 1})
      java-map (doto (java.util.HashMap.) (.put :value 1))]
  (assert (map-entry? entry))
  (doseq [value [nil [] [:value 1] entry java-map (transient {:value 1})
                 false true 0 1N 1.5 "text" :value 'value \v]]
    (assert (false? (map? value)))))

;; A Java class can be both numeric and an IPersistentMap.
(let [number-map (proxy [Number IPersistentMap] []
                   (intValue [] 7)
                   (longValue [] 7)
                   (floatValue [] 7.0)
                   (doubleValue [] 7.0))
      integer-map (proxy [java.math.BigInteger IPersistentMap] ["7"])]
  (assert (and (number? number-map) (map? number-map)))
  (assert (and (integer? integer-map) (map? integer-map)))
  (assert (not (int? integer-map))))

(prn {:clojure (clojure-version) :checked-map-predicate :ok})

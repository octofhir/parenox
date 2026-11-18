(ns demo.reduce-kv (:require [clojure.spec.alpha :as s]))

(defn total-usage
  {:t/sig '[(s/map-of keyword? number?) :-> number?]}
  [quotas]
  (reduce-kv (fn [total _ units] (+ total units)) 0 quotas))

(defn config-score
  {:t/sig '[{:count number? :label string?} :-> number?]}
  [config]
  (reduce-kv (fn [total field value]
               (case field
                 :count (+ total value)
                 :label (+ total (count value))))
             0 config))

(defn weighted-total
  {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]}
  [values]
  (reduce-kv (fn [total index value] (+ total (* (inc index) value))) 0 values))

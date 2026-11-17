(ns demo.dictionaries-bad
  (:require [clojure.spec.alpha :as s]))

(s/def ::prices (s/map-of string? number?))
(s/def ::counts (s/map-of keyword? number?))

(defn increment-sku
  {:t/sig '[::prices :-> (s/coll-of number? :kind vector?)]}
  [prices]
  (mapv (fn [[sku _]] (inc sku)) prices))

(defn record-open
  {:t/sig '[::counts :-> ::counts]}
  [counts]
  (assoc counts :open "unknown"))

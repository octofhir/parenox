(ns demo.dictionaries
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def ::prices (s/map-of string? number?))
(s/def ::counts (s/map-of keyword? number?))

(defn price-labels
  {:t/sig '[::prices :-> (s/coll-of string? :kind vector?)]}
  [prices]
  (mapv (fn [[sku price]] (str (str/upper-case sku) ": " price)) prices))

(defn next-open-count
  {:t/sig '[::counts :-> number?]}
  [counts]
  (if (contains? counts :open)
    (inc (get counts :open))
    1))

(defn record-open
  {:t/sig '[::counts :-> ::counts]}
  [counts]
  (update counts :open (fnil inc 0)))

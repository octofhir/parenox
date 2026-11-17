(ns demo.dynamic-paths
  (:require [clojure.spec.alpha :as s]))

(s/def ::prices (s/map-of keyword? number?))
(s/def ::catalog (s/map-of string? ::prices))
(s/def ::price number?)
(s/def ::stock number?)
(s/def ::item (s/keys :req-un [::price ::stock]))
(s/def ::inventory (s/map-of string? ::item))

(defn quote-price
  {:t/sig '[::catalog string? keyword? number? :-> number?]}
  [catalog sku tier fallback]
  (get-in catalog [sku tier] fallback))

(defn reprice
  {:t/sig '[::catalog string? number? :-> ::catalog]}
  [catalog sku price]
  (assoc-in catalog [sku :retail] price))

(defn adjust-price
  {:t/sig '[::catalog string? number? :-> ::catalog]}
  [catalog sku delta]
  (update-in catalog [sku :retail] (fnil + 0) delta))

(defn raise-current-price
  {:t/sig '[::inventory string? :-> ::inventory]}
  [inventory sku]
  (if (contains? inventory sku)
    (update-in inventory [sku :price] inc)
    inventory))

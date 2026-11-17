(ns demo.dynamic-paths-bad
  (:require [clojure.spec.alpha :as s]))

(s/def ::catalog (s/map-of string? (s/map-of keyword? number?)))
(s/def ::flat-prices (s/map-of string? number?))

(defn raise-price
  {:t/sig '[::catalog string? :-> ::catalog]}
  [catalog sku]
  (update-in catalog [sku :retail] inc))

(defn add-retail-tier
  {:t/sig '[::flat-prices string? :-> map?]}
  [prices sku]
  (assoc-in prices [sku :retail] 120))

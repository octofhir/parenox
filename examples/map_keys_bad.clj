(ns demo.map-keys-bad
  (:require [clojure.spec.alpha :as s]))

(s/def ::prices (s/map-of string? number?))
(s/def ::optional-prices (s/map-of string? (s/nilable number?)))

(defn next-price
  {:t/sig '[::optional-prices string? :-> number?]}
  [prices sku]
  (if (contains? prices sku)
    (inc (get prices sku))
    0))

(defn mark-pending
  {:t/sig '[::prices string? :-> ::prices]}
  [prices sku]
  (assoc prices sku "pending"))

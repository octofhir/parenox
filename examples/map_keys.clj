(ns demo.map-keys
  (:require [clojure.spec.alpha :as s]))

(s/def ::prices (s/map-of string? number?))

(defn quoted-price
  {:t/sig '[::prices string? number? :-> number?]}
  [prices sku fallback]
  (get prices sku fallback))

(defn discounted-price
  {:t/sig '[::prices string? number? :-> number?]}
  [prices sku discount]
  (if (contains? prices sku)
    (- (get prices sku) discount)
    0))

(defn adjust-price
  {:t/sig '[::prices string? number? :-> ::prices]}
  [prices sku delta]
  (update prices sku (fnil + 0) delta))

(defn catalog-price
  {:t/sig '[:-> number?]}
  []
  (get-in {:catalog {"SKU-17" {:price 120}}} [:catalog "SKU-17" :price]))

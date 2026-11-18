(ns demo.ordered-paths
  (:require [clojure.spec.alpha :as s]))

(s/def ::prices (s/map-of keyword? number?))
(s/def ::catalog (s/map-of string? ::prices))
(s/def ::route (s/tuple string? keyword?))

(defn quote-price
  {:t/sig '[::catalog ::route number? :-> number?]}
  [catalog path fallback]
  (get-in catalog path fallback))

(defn retail-quote
  {:t/sig '[::catalog string? :-> number?]}
  [catalog sku]
  (let [path [sku :retail]]
    (quote-price catalog path 0)))

(defn adjust-both-prices
  {:t/sig '[::catalog string? number? number? :-> ::catalog]}
  [catalog sku wholesale delta]
  (let [wholesale-path [sku :wholesale]
        retail-path [sku :retail]]
    (-> catalog
        (assoc-in wholesale-path wholesale)
        (update-in retail-path (fnil + 0) delta))))

(defn price-label
  {:t/sig '[(s/tuple string? number?) :-> string?]}
  [[sku price]]
  (str sku ": " price))

(defn computed-quote
  {:t/sig '[::catalog string? :-> number?]}
  [catalog sku]
  (get-in catalog (list sku :retail) 0))

(defn adjust-stored-path
  {:t/sig '[::catalog string? number? :-> ::catalog]}
  [catalog sku price]
  (let [route (cons sku '(:retail))]
    (-> catalog
        (assoc-in route price)
        (update-in route (fnil inc 0)))))

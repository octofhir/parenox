(ns demo.ordered-paths-bad
  (:require [clojure.spec.alpha :as s]))

(defn route-sku
  {:t/sig '[(s/tuple string? keyword?) :-> string?]}
  [[sku _]]
  sku)

(defn build-route
  {:t/sig '[string? :-> string?]}
  [sku]
  (route-sku [:retail sku]))

(defn increment-quantity
  {:t/sig '[:-> number?]}
  []
  (let [[_ quantity] ["SKU-17"]]
    (inc quantity)))

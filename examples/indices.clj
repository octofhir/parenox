(ns demo.indices
  (:require [clojure.spec.alpha :as s]))

(s/def ::row (s/tuple string? number?))
(s/def ::price number?)
(s/def ::item (s/keys :req-un [::price]))
(s/def ::items (s/coll-of ::item :kind vector?))
(s/def ::invoice (s/keys :req-un [::items]))

(defn row-price
  {:t/sig '[::row :-> number?]}
  [row]
  (let [price-index 1]
    (nth row price-index)))

(defn first-item-price
  {:t/sig '[::invoice number? :-> number?]}
  [invoice fallback]
  (let [path [:items 0 :price]]
    (get-in invoice path fallback)))

(defn first-rate
  {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]}
  [rates]
  (get rates 0 0))

(ns demo.map-builders (:require [clojure.spec.alpha :as s]))

(defn public-reading
  {:t/sig '[(s/tuple number? string?) :-> {"amount" number? "unit" string?}]}
  [[amount unit]]
  (-> (zipmap [:amount :unit] [amount unit])
      (update-keys (fn [key]
                     (case key :amount "amount" :unit "unit")))))

(defn totals
  {:t/sig '[(s/coll-of keyword? :kind vector?) (s/coll-of number? :kind vector?)
            :-> (s/map-of keyword? number?)]}
  [names amounts]
  (zipmap names amounts))

(defn statuses
  {:t/sig '[(s/map-of #{:pending :done} boolean?)
            :-> (t/map {:opt {"pending" boolean? "done" boolean?}})]}
  [flags]
  (update-keys flags (fn [key] (case key :pending "pending" :done "done"))))

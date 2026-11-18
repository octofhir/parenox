(ns demo.merge-with (:require [clojure.spec.alpha :as s]))

(defn sum-quotas
  {:t/sig '[(s/map-of keyword? number?) (s/map-of keyword? number?)
            :-> (s/map-of keyword? number?)]}
  [previous current]
  (merge-with + previous current))

(defn join-labels
  {:t/sig '[(t/map {:req {:id string?} :opt {:label string?} :extra :open})
            (t/map {:opt {:label string?}})
            :-> (t/map {:req {:id string?} :opt {:label string?} :extra :open})]}
  [stored labels]
  (merge-with (fn [a b] (str a " / " b)) stored labels))

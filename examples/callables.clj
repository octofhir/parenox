(ns demo.callables
  (:require [clojure.spec.alpha :as s]))

(defn priorities
  {:t/sig '[:-> (s/coll-of (s/nilable number?) :kind vector?)]}
  []
  (mapv {:high 3} [:high :low]))

(defn statuses
  {:t/sig '[:-> (s/coll-of (s/nilable boolean?) :kind vector?)]}
  []
  (mapv :active [{:active true} {:active false} {}]))

(defn selected-label {:t/sig '[boolean? :-> string?]} [translated?]
  (let [read-label (if translated? :localized :label)]
    (read-label {:label "Ready" :localized "Prêt"})))

(defn explicit-nil {:t/sig '[:-> nil?]} []
  ({:label nil} :label "Fallback"))

(defn safe-label {:t/sig '[:-> string?]} []
  (:label 42 "Fallback"))

(defn choose-stage {:t/sig '[:-> string?]} []
  (["draft" "ready"] 1))

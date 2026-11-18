(ns demo.group-by (:require [clojure.spec.alpha :as s]))

(defn tasks-by-status
  {:t/sig '[(s/coll-of {:status #{:queued :done} :id number?} :kind vector?)
            :-> (s/map-of #{:queued :done}
                          (s/coll-of {:status #{:queued :done} :id number?} :kind vector?))]}
  [tasks]
  (group-by :status tasks))

(defn labels-by-kind
  {:t/sig '[:-> {:text (s/tuple #{"first"} #{"last"})
                 :flag (s/tuple #{false})}]}
  []
  (group-by (fn [value] (if (string? value) :text :flag))
            ["first" false "last"]))

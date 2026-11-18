(ns map-boundary.response
  (:require [clojure.spec.alpha :as s]))

(defn select-name
  {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :name a) :ret b)
                        (s/map-of keyword? a) :-> b])}
  [project row]
  (project (req! row :display-name)))

(defn ok
  {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :row a) :ret {:body b})
                        {:row a} :-> {:status number? :body b}])}
  [render result]
  (assoc (render (:row result)) :status 200))

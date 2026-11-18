(ns demo.update-vals
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(defn normalize-labels
  {:t/sig '[(s/map-of keyword? (s/nilable string?)) :-> (s/map-of keyword? string?)]}
  [labels]
  (update-vals labels (fnil str/upper-case "Untitled")))

(defn advance
  {:t/sig '[(t/map {:req {:count number?} :opt {:label string?}})
            :-> (t/map {:req {:count number?} :opt {:label string?}})]}
  [row]
  (update-vals row #(if (string? %) (str/trim %) (inc %))))

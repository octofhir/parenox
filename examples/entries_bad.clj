(ns demo.entries-bad
  (:require [clojure.spec.alpha :as s]))

(defn increment-counts
  {:t/sig '[:-> (s/coll-of number? :kind vector?)]}
  []
  (mapv (fn [[_ total]] (inc total)) {:admitted "12"}))

(defn key-of-pair
  {:t/sig '[:-> keyword?]}
  []
  (key [:admitted 12]))

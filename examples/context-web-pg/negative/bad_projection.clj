(ns bad-projection
  (:require [clojure.spec.alpha :as s]))

(defn bad-projection
  {:t/sig '[(s/coll-of string? :kind seq?) :-> number?]}
  [fields]
  (if (empty? fields)
    0
    (loop [[field & more] fields]
      (if (empty? more)
        (inc field)
        (recur more)))))

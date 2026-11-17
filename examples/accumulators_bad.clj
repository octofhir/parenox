(ns demo.accumulators-bad
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(defn unchecked-labels
  {:t/sig '[(s/coll-of (s/nilable string?) :kind vector?)
            :-> (s/coll-of string? :kind vector?)]} [names]
  (loop [remaining (seq names) output []]
    (if remaining
      (recur (next remaining) (conj output (str/upper-case (first remaining))))
      output)))

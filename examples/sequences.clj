(ns demo.sequences
  (:require [clojure.spec.alpha :as s]))

(s/def ::amounts (s/coll-of number? :kind vector?))

(defn sum-amounts {:t/sig '[::amounts :-> number?]} [amounts]
  (loop [remaining (seq amounts) total 0]
    (if-some [items remaining]
      (recur (next items) (+ total (first items)))
      total)))

(defn second-label {:t/sig '[:-> string?]} []
  (first (next [1 "second"])))

(defn positional-label {:t/sig '[:-> string?]} []
  (let [[_ label] (seq [1 "label"])]
    label))

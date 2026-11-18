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

(defn accumulate-with-rest {:t/sig '[::amounts :-> number?]} [amounts]
  (loop [remaining amounts total 0]
    (if-let [items (seq remaining)]
      (recur (rest items) (+ total (first items)))
      total)))

(defn sum-with-empty-guard {:t/sig '[::amounts :-> number?]} [amounts]
  (loop [items (seq amounts) total 0]
    (if-not (empty? items)
      (recur (next items) (+ total (first items)))
      total)))

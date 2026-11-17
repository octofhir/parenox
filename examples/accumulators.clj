(ns demo.accumulators
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def ::names (s/coll-of (s/nilable string?) :kind vector?))
(s/def ::labels (s/coll-of string? :kind vector?))
(s/def ::amounts (s/coll-of number? :kind vector?))

(defn labels {:t/sig '[::names :-> ::labels]} [names]
  (loop [remaining (seq names) output []]
    (if remaining
      (recur (next remaining)
             (conj output (str/upper-case (or (first remaining) "Unknown"))))
      output)))

(defn project-values
  {:t/sig '[::amounts :-> (s/tuple ::amounts ::labels)]} [amounts]
  (loop [remaining (seq amounts) values [] labels []]
    (if remaining
      (recur (next remaining)
             (conj values (first remaining))
             (conj labels (str (first remaining))))
      [values labels])))

(defn collect-once {:t/sig '[:-> (s/tuple number?)]} []
  (loop [start? true output []]
    (if start?
      (recur false (conj output 42))
      output)))

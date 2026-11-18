(ns demo.reductions
  (:require [clojure.spec.alpha :as s]))

(defn sum-values {:t/sig '[:-> number?]} []
  (reduce + [1 2 3]))

(defn total-in-map {:t/sig '[:-> number?]} []
  (:total
   (reduce (fn [state delta] (update state :total + delta))
           {:total 0}
           [1 2 3])))

(defn pick-collected {:t/sig '[:-> string?]} []
  (nth (reduce conj [] [1 "last"]) 1))

(defn append-labels {:t/sig '[:-> string?]} []
  (reduce (fn [acc label] (str acc "-" label))
          "a"
          (next [nil "b" "c"])))

(defn empty-count {:t/sig '[:-> number?]} []
  (reduce + (rest [1])))

(defn sum-open
  {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]}
  [values]
  (reduce + 0 values))

(defn append-open
  {:t/sig '[(s/coll-of string? :kind vector?) :-> (s/coll-of string? :kind vector?)]}
  [labels]
  (reduce conj ["default"] labels))

(defn sum-before-label {:t/sig '[:-> number?]} []
  (reduce (fn [total value]
            (if (string? value)
              (reduced total)
              (+ total value)))
          0
          [2 3 "stop" false]))

(defn sum-line-items {:t/sig '[:-> number?]} []
  (reduce (fn [total entry] (+ total (val entry)))
          0
          {:base 5 :tax 2}))

(defn total-utf16-units {:t/sig '[:-> number?]} []
  (reduce (fn [total unit]
            (if (char? unit) (inc total) total))
          0
          "A😀"))

(defn sum-distinct {:t/sig '[:-> number?]} []
  (reduce + 0 #{2 3 5}))

(defn singleton-price {:t/sig '[:-> number?]} []
  (val (reduce nil {:price 7})))

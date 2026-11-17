(ns demo.cond
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(defn label {:t/sig '[(s/nilable (s/or :text string? :number number?)) :-> string?]}
  [input]
  (cond
    (nil? input) "MISSING"
    (string? input) (str/upper-case input)
    (number? input) (str (inc input))))

(defn loop-label {:t/sig '[string? :-> string?]} [input]
  (loop [pending? true label nil]
    (cond
      pending? (recur false (str/trim input))
      :else label)))

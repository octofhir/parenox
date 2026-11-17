(ns demo.cond-bad
  (:require [clojure.spec.alpha :as s]))

(defn missing-label {:t/sig '[(s/nilable string?) :-> string?]} [input]
  (cond
    (string? input) input))

(defn increment-wrong-branch {:t/sig '[(s/or :text string? :number number?) :-> number?]}
  [input]
  (cond
    (string? input) (inc input)
    :else (inc input)))

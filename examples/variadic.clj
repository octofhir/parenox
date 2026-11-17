(ns demo.variadic
  (:require [clojure.spec.alpha :as s]))

(defn adjust
  {:t/sig '[[number? :-> number?]
            [number? & number? :-> number?]]}
  ([base] base)
  ([base & deltas] (+ base (first deltas))))

(defn first-label
  {:t/sig '[boolean? & string? :-> (s/nilable string?)]}
  [skip? & labels]
  (if skip?
    (recur false (next labels))
    (first labels)))

(defn inline-offset {:t/sig '[:-> number?]} []
  ((fn [base & [delta]] (+ base delta)) 4 3))

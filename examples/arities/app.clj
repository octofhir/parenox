(ns demo.arities.app
  (:require [demo.arities.provider :as provider]))

(defn next-offset {:t/sig '[number? :-> number?]} [offset]
  (provider/count-offset offset))

(defn label-size {:t/sig '[string? :-> number?]} [label]
  (let [describe provider/describe]
    (describe label)))

(defn ready-label {:t/sig '[:-> string?]} []
  (provider/describe))

(defn label-widths
  {:t/sig '[(clojure.spec.alpha/coll-of string? :kind vector?)
           :-> (clojure.spec.alpha/coll-of number? :kind vector?)]}
  [labels]
  (mapv (fn
          ([label] (count label))
          ([label offset] (+ (count label) offset)))
        labels))

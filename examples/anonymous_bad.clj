(ns demo.anonymous-bad
  (:require [clojure.spec.alpha :as s]))

(defn increment-labels {:t/sig '[(s/coll-of string? :kind vector?)
                               :-> (s/coll-of number? :kind vector?)]} [labels]
  (mapv #(inc %) labels))

(defn missing-slot {:t/sig '[:-> number?]} []
  (#(inc %2) 3))

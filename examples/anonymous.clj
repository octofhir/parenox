(ns demo.anonymous
  (:require [clojure.spec.alpha :as s]))

(defn offsets
  {:t/sig '[(s/coll-of number? :kind vector?) :-> (s/coll-of number? :kind vector?)]}
  [points]
  (mapv #(+ % %1) points))

(defn label-id {:t/sig '[number? string? :-> string?]} [id label]
  (#(str %2 ":" %1) id label))

(defn first-extra {:t/sig '[:-> string?]} []
  (apply #(first %&) ["ready" "later"]))

(defn settle {:t/sig '[:-> string?]} []
  (#(if % (recur false) "settled") true))

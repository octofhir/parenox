(ns demo.vectors-bad
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(defn increment-counts
  {:t/sig '[:-> (s/coll-of number? :kind vector?)]}
  []
  (mapv inc ["2"]))

(defn uppercase-names
  {:t/sig '[(s/coll-of (s/nilable string?) :kind vector?) :-> (s/coll-of string? :kind vector?)]}
  [names]
  (mapv (fn [name] (str/upper-case name)) names))

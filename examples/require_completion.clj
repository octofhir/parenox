(ns require-completion
  (:require [clojure.string :as str]))

(require '[clojure.set :as set])

(defn join-names [names]
    (str/j)
  (str/join ", " names))

(defn merge-tags [left right]
  (set/union left right))

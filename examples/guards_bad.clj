(ns demo.guards-bad
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(defn display-name
  {:t/sig '[(s/nilable string?) :-> string?]}
  [name]
  (when-let [value name]
    (str/upper-case value)))

(defn next-value
  {:t/sig '[:-> number?]}
  []
  (if-some [value false]
    (inc value)
    0))

(ns demo.conditional
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(defn display-name
  {:t/sig '[string? boolean? :-> string?]}
  [name suffix?]
  (cond-> name
    true str/trim
    true str/upper-case
    suffix? (str "!")))

(defn prefixed-name
  {:t/sig '[string? boolean? :-> string?]}
  [name prefix?]
  (cond->> name
    prefix? (str "Patient: ")
    true str/upper-case))

(defn defaulted-name
  {:t/sig '[(s/nilable string?) :-> string?]}
  [name]
  (cond-> name
    (nil? name) (or "Unknown")
    true str/upper-case))

(defn flexible-name
  {:t/sig '[string? number? :-> string?]}
  [name number]
  (as-> name value
    (str/trim value)
    (str number ": " value)
    (str/upper-case value)))

(defn after-nil
  {:t/sig '[:-> string?]}
  []
  (cond-> nil
    true (str "A")
    true (str "B")))

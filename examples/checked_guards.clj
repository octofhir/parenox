(ns demo.checked-guards
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(defn enabled-text?
  {:t/sig '[boolean? (s/nilable string?) :-> boolean?]
   :t/guard '[:then value string?]}
  [enabled value]
  (and enabled (string? value)))

(defn text?
  {:t/sig '[(s/nilable string?) :-> boolean?]
   :t/guard '[:both value string?]}
  [value]
  (if (string? value) true false))

(defn label {:t/sig '[boolean? (s/nilable string?) :-> string?]} [enabled value]
  (if (enabled-text? enabled value)
    (str/upper-case value)
    "Unknown"))

(defn missing-label {:t/sig '[(s/nilable string?) :-> nil?]} [value]
  (let [has-text? text?]
    (if (has-text? value) nil value)))

(ns demo.conditional-bad
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

;; If the condition is false, the original Number remains in the result.
(defn mixed-result
  {:t/sig '[boolean? :-> number?]}
  [text?]
  (cond-> 1 text? str))

;; Named threading continues with Nil; handle it before upper-case.
(defn unsafe-name
  {:t/sig '[(s/nilable string?) :-> string?]}
  [name]
  (as-> name value
    (str/upper-case value)))

;; A selected step receives the current String as inc's argument.
(defn bad-number
  {:t/sig '[string? :-> number?]}
  [name]
  (cond-> name true inc))

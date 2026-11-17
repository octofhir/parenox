(ns demo
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def :patient/id string?)
(s/def :patient/name string?)
(s/def ::patient
  (s/keys :req-un [:patient/id]
          :opt-un [:patient/name]))

;; Optional field: String | Nil cannot be passed to upper-case.
(defn display-name
  {:t/sig '[::patient :-> string?]}
  [patient]
  (str/upper-case (:name patient)))

;; A declared Number return cannot be a String.
(defn increment
  {:t/sig '[number? :-> number?]}
  [n]
  "oops")

;; Calls are checked against the declared signature.
(defn bad-call
  {:t/sig '[:-> number?]}
  []
  (increment "not a number"))

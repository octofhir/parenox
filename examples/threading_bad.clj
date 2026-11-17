(ns demo.threading-bad
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def :patient/name string?)
(s/def ::patient (s/keys :opt-un [:patient/name]))

(defn unsafe-name
  {:t/sig '[::patient :-> string?]}
  [patient]
  (-> patient :name str/upper-case))

(defn bad-number
  {:t/sig '[:-> number?]}
  []
  (-> "not a number" inc))

(defn optional-result
  {:t/sig '[(s/nilable string?) :-> string?]}
  [name]
  (some-> name str/upper-case))

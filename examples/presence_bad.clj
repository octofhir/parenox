(ns demo.presence-bad
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def :patient/name (s/nilable string?))
(s/def ::patient (s/keys :opt-un [:patient/name]))

(defn nullable-name
  {:t/sig '[::patient :-> string?]}
  [patient]
  (if (contains? patient :name)
    (str/upper-case (:name patient))
    "UNKNOWN"))

(defn missing-name
  {:t/sig '[::patient :-> (s/nilable string?)]}
  [patient]
  (if (contains? patient :name)
    (:name patient)
    (req! patient :name)))

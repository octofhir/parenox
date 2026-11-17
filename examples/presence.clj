(ns demo.presence
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def :patient/name string?)
(s/def :patient/active boolean?)
(s/def ::patient (s/keys :opt-un [:patient/name :patient/active]))

(defn display-name
  {:t/sig '[(s/nilable ::patient) :-> string?]}
  [patient]
  (if (contains? patient :name)
    (str/upper-case (req! patient :name))
    "UNKNOWN"))

(defn status
  {:t/sig '[::patient :-> string?]}
  [patient]
  (if (contains? patient :active)
    (if (:active patient) "active" "inactive")
    "unknown"))

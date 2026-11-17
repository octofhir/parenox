(ns demo
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def :patient/id string?)
(s/def :patient/name string?)
(s/def ::patient
  (s/keys :req-un [:patient/id]
          :opt-un [:patient/name]))

(defn display-name
  {:t/sig '[::patient :-> string?]}
  [patient]
  (str/upper-case (or (:name patient) "Unknown")))

(defn guarded-name
  {:t/sig '[::patient :-> string?]}
  [{:keys [name]}]
  (if (some? name)
    (str/upper-case name)
    "Unknown"))

(defn increment
  {:t/sig '[number? :-> number?]}
  [n]
  (+ n 1))

(defn call-increment
  {:t/sig '[number? :-> number?]}
  [n]
  (increment n))

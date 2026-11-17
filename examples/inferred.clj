(ns demo.inferred
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def :patient/name string?)
(s/def ::patient (s/keys :opt-un [:patient/name]))

;; Both private helpers infer their inputs and results from the typed caller.
(defn- normalize-name [name]
  (str/upper-case (or name "Unknown")))

(defn- patient-label [{:keys [name]}]
  (normalize-name name))

(defn display-name
  {:t/sig '[::patient :-> string?]}
  [patient]
  (patient-label patient))

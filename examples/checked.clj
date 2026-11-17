(ns demo.checked
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def :patient/id string?)
(s/def :patient/name string?)
(s/def ::patient (s/keys :req-un [:patient/id] :opt-un [:patient/name]))
(s/def :request/patient ::patient)
(s/def ::request (s/keys :opt-un [:request/patient]))

;; Missing :name throws at runtime; after successful destructuring name is a String.
(defn display-name
  {:t/sig '[::patient :-> string?]}
  [{:keys! [id name]}]
  (str/upper-case name))

;; Renamed required binding plus an ordinary optional binding.
(defn label
  {:t/sig '[::patient :-> string?]}
  [{:req! {patient-id :id} :keys [name]}]
  (str patient-id ": " (or name "Unknown")))

;; Each checked level keeps its own presence check.
(defn request-id
  {:t/sig '[::request :-> string?]}
  [{:req! {{:keys! [id]} :patient}}]
  id)

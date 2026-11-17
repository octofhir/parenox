(ns demo.updates
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def :patient/name string?)
(s/def :patient/visits number?)
(s/def ::patient (s/keys :opt-un [:patient/name :patient/visits]))
(s/def :request/patient ::patient)
(s/def ::request (s/keys :opt-un [:request/patient]))

(defn visits-after
  {:t/sig '[::patient number? :-> number?]}
  [patient delta]
  (:visits (update patient :visits (fnil + 0) delta)))

(defn display-name
  {:t/sig '[::request :-> string?]}
  [request]
  (let [fallback "Unknown"]
    (-> request
        (update-in [:patient :name] (fn [name] (str/upper-case (or name fallback))))
        (get-in [:patient :name]))))

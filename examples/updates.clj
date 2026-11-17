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
  (let [updated (if (contains? patient :visits)
                  (update patient :visits + delta)
                  (assoc patient :visits delta))]
    (:visits updated)))

(defn- normalize-or-default [name fallback]
  (str/upper-case (or name fallback)))

(defn display-name
  {:t/sig '[::request :-> string?]}
  [request]
  (-> request
      (update-in [:patient :name] normalize-or-default "Unknown")
      (get-in [:patient :name])))

(ns clinic.http
  (:require [clojure.string :as str]
            [clinic.model :as model]
            [clinic.store :as store]
            [clinic.service :as service]))

(defn show-patient
  {:t/sig '[::model/request :-> ::model/response]}
  [request]
  (let [id (get-in request [:path-params :id])
        ;; Deliberate mistake: the repository expects a patient id, not a number.
        patient (store/find-patient 42)]
    ;; Deliberate mistake: this library contract expects a string.
    (if (str/starts-with? 42 "p-")
      (if patient
        {:status 200 :body (service/display-name patient)}
        {:status 404 :body "Patient not found"})
      {:status 400 :body "Invalid patient id"})))

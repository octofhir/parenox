(ns map-boundary.http
  (:require [clojure.spec.alpha :as s]
            [map-boundary.model :as model]
            [map-boundary.response :as response]
            [vendor.patient-store :as store]))

(defn- render-name {:t/sig '[::model/row :-> {:body (s/nilable string?)}]} [row]
  {:body (if (contains? row :display-name)
           (response/select-name (fn [label] label) row)
           "Unknown")})

(defn show-patient
  {:t/sig '[{:id string?} :-> {:status number? :body (s/nilable string?)}]}
  [{:keys [id]}]
  (if-some [result (store/fetch id)]
    (response/ok render-name result)
    {:status 404 :body nil}))

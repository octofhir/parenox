(ns demo.paths
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def :patient/name string?)
(s/def :request/patient (s/keys :opt-un [:patient/name]))
(s/def ::request (s/keys :opt-un [:request/patient]))

(defn display-name
  {:t/sig '[::request :-> string?]}
  [request]
  (str/upper-case (get-in request [:patient :name] "Unknown")))

(defn normalized-name
  {:t/sig '[::request string? :-> string?]}
  [request name]
  (-> request
      (assoc-in [:patient :name] (str/trim name))
      (get-in [:patient :name])))

(ns clinic.model
  (:require [clojure.spec.alpha :as s]))

(s/def :patient/id string?)
(s/def :patient/name string?)
(s/def ::patient (s/keys :req-un [:patient/id] :opt-un [:patient/name]))
(s/def :request/path-params (s/keys :req-un [:patient/id]))
(s/def ::request (s/keys :req-un [:request/path-params]))
(s/def :response/status number?)
(s/def :response/body string?)
(s/def ::response (s/keys :req-un [:response/status :response/body]))

(ns demo.defaults
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def :patient/id string?)
(s/def :patient/name string?)
(s/def ::patient (s/keys :req-un [:patient/id] :opt-un [:patient/name]))

;; An absent name uses the fallback. The id stays a checked binding.
(defn display-name
  {:t/sig '[::patient :-> string?]}
  [{:keys! [id] :keys [name] :or {name "Unknown"}}]
  (str/upper-case name))

;; Defaults can name a later ordinary function argument.
(defn configured-name
  {:t/sig '[::patient string? :-> string?]}
  [{:keys [name] :or {:name fallback}} fallback]
  name)

(defn- fallback-name [] "Unknown")

;; A private helper called in a parameter default participates in cache invalidation.
(defn inferred-name
  {:t/sig '[::patient :-> string?]}
  [{:keys [name] :or {name (fallback-name)}}]
  name)

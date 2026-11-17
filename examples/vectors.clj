(ns demo.vectors
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def :patient/name (s/nilable string?))
(s/def ::patient (s/keys :opt-un [:patient/name]))
(s/def ::patients (s/coll-of ::patient :kind vector?))
(s/def ::labels (s/coll-of string? :kind vector?))
(s/def ::counts (s/coll-of number? :kind vector?))
(s/def ::optional-counts (s/coll-of (s/nilable number?) :kind vector?))

(defn display-labels
  {:t/sig '[::patients :-> ::labels]}
  [patients]
  (let [prefix "Patient "]
    (->> patients
         (mapv (fn [{:keys [name]}] name))
         (filterv some?)
         (mapv (fn [name] (str prefix (str/upper-case name)))))))

(defn adjust-counts
  {:t/sig '[::optional-counts ::counts :-> ::counts]}
  [counts deltas]
  (mapv (fnil + 0) counts deltas))

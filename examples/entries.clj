(ns demo.entries
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(defn display-labels
  {:t/sig '[string? string? :-> (s/coll-of string? :kind vector?)]}
  [patient clinician]
  (->> {:patient patient :clinician clinician}
       (filterv (fn [entry] (not (= "" (val entry)))))
       (mapv (fn [[role name]]
               (str role " " (str/upper-case name))))))

(defn recorded-roles
  {:t/sig '[string? :-> (s/coll-of keyword? :kind vector?)]}
  [name]
  (mapv key {:patient name}))

;; A present nil label is an explicit choice. Only an absent key gets the fallback.
(defn preferred-label
  {:t/sig '[(t/map {:opt {:label (s/nilable string?)} :extra :open})
            :-> (s/nilable string?)]}
  [profile]
  (if-let [entry (find profile :label)]
    (val entry)
    "Unlabeled"))

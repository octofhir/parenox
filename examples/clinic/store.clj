(ns clinic.store
  (:require [clojure.spec.alpha :as s]
            [clinic.model :as model]))

;; A small deterministic repository stand-in; a missing patient is represented by nil.
(defn find-patient
  {:t/sig '[:patient/id :-> (s/nilable ::model/patient)]}
  [id]
  (if (= id "p-1")
    {:id id :name "Alice"}
    nil))

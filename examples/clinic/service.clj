(ns clinic.service
  (:require [clojure.string :as str]
            [clinic.model :as model]))

;; Deliberate mistake: the model allows a missing name.
(defn display-name
  {:t/sig '[::model/patient :-> string?]}
  [patient]
  (str/upper-case (:name patient)))

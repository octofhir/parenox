(ns demo.updates-bad
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def :patient/visits number?)
(s/def ::patient (s/keys :opt-un [:patient/visits]))

(defn increment-visits
  {:t/sig '[::patient :-> number?]}
  [patient]
  (:visits (update patient :visits inc)))

(defn normalize-name
  {:t/sig '[:-> string?]}
  []
  (get-in (update-in {:patient {:name nil}} [:patient :name] str/trim)
          [:patient :name]))

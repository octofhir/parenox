(ns demo.map-sequences (:require [clojure.spec.alpha :as s]))

(s/def ::quotas (s/map-of keyword? number?))

(defn total-quota {:t/sig '[::quotas :-> number?]} [quotas]
  (reduce + 0 (vals quotas)))

(defn first-service {:t/sig '[::quotas :-> (s/nilable keyword?)]} [quotas]
  (first (keys quotas)))

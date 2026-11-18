(ns demo.split-externs
  (:require [vendor.clinic :as clinic]))

(defn patient-label {:t/sig '[string? :-> string?]} [id]
  (clinic/render-label (clinic/find-patient id)))

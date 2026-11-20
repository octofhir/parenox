(ns portable.namespace-probe
  (:require [portable.loading :as loaded :refer [message]]))

(require '[portable.loading :as repeated])

(println "namespace ready")
(prn [(loaded/message :alias) (message :refer) (repeated/message :require)])

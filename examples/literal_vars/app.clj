(ns demo.literal-vars.app
  (:require [demo.literal-vars.config :as config :refer [status]]))

(defn next-limit {:t/sig '[:-> number?]} []
  (inc (:limit config/default-plan)))

(defn plan-mode {:t/sig '[:-> #{::config/online}]} []
  (:mode config/default-plan))

(defn status-code {:t/sig '[:-> #{:active}]} []
  status)

(ns demo.keyword-aliases
  (:require [demo.schema :as-alias schema]))

(defn status-key {:t/sig '[:-> #{:demo.schema/status}]} []
  ::schema/status)

(defn local-key {:t/sig '[:-> #{:demo.keyword-aliases/ready}]} []
  ::ready)

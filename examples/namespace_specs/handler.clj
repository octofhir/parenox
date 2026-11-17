(ns demo.namespace-specs.handler
  (:require [clojure.spec.alpha :as s]
            [demo.namespace-specs.config :as config]))

(s/def ::id string?)
(s/def :wire/id ::id)

(defn header {:t/sig '[:-> keyword?]} []
  (config/request-key))

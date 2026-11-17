(ns demo.namespace-specs.config
  (:require [demo.namespace-specs.handler :as-alias handler]))

(defn request-key {:t/sig '[:-> #{::handler/request-id}]} []
  ::handler/request-id)

(defn keep-id {:t/sig '[::handler/id :-> ::handler/id]} [id]
  id)

(ns demo.namespaced-keys
  (:require [catalog.payload :as-alias payload]))

(defn sku {:t/sig '[:-> #{"A-1"}]} []
  (:catalog.payload/sku #::payload{:sku "A-1" :_/source "cache"}))

(defn access-level {:t/sig '[:-> #{:public}]} []
  (::level #::{:level :public}))

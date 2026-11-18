(ns map-boundary.model
  (:require [clojure.spec.alpha :as s]))

(s/def ::row (s/map-of keyword? (s/nilable string?)))

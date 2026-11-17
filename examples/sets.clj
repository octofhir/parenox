(ns demo.sets
  (:require [clojure.spec.alpha :as s]))

;; In a spec declaration, a set describes the allowed values of one status.
(s/def ::status #{"active" "inactive"})

(defn default-status
  {:t/sig '[:-> ::status]}
  []
  (let [status "active"] status))

;; In an expression, the same syntax creates a Clojure set, not a map or scalar.
(defn statuses
  {:t/sig '[:-> (s/coll-of ::status :kind set?)]}
  []
  #{"active" "inactive"})

(defn status-count
  {:t/sig '[:-> number?]}
  []
  (count (statuses)))

;; Clojure sets are callable: they return a member or nil, not true/false.
(defn find-status
  {:t/sig '[string? :-> (s/nilable string?)]}
  [value]
  ((statuses) value))

(ns demo.macro-scopes
  (:require [vendor.bindings :as vendor]))

(defn ordinary {:t/sig '[string? :-> string?]} [label]
  (let [label (str label "!") result label]
    result))

(defn with-scope {:t/sig '[string? :-> string?]} [label]
  (vendor/with-bindings [label (str label "!") result label]
    result))

(ns demo.defaults-bad
  (:require [clojure.string :as str]))

;; A default never replaces a present Nil.
(defn nil-name
  {:t/sig '[:-> string?]}
  []
  (let [{:keys [name] :or {name "Unknown"}} {:name nil}]
    (str/upper-case name)))

;; Defaults are evaluated even if the key is present.
(defn invalid-fallback
  {:t/sig '[:-> string?]}
  []
  (let [{:keys [name] :or {name (inc "bad")}} {:name "Ada"}]
    name))

;; A required key cannot have a default, even when it is present.
(defn conflicting-default
  {:t/sig '[:-> string?]}
  []
  (let [{:keys! [name] :or {name "Unknown"}} {:name "Ada"}]
    name))

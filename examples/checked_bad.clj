(ns demo.checked-bad
  (:require [clojure.string :as str]))

;; A closed literal proves that :name is absent.
(defn missing-name
  {:t/sig '[:-> string?]}
  []
  (let [{:keys! [id name]} {:id "p-1"}]
    name))

;; Presence succeeds here, but Nil cannot be passed to upper-case.
(defn nil-name
  {:t/sig '[:-> string?]}
  []
  (let [{:keys! [name]} {:name nil}]
    (str/upper-case name)))

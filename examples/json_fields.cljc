(ns demo.json-fields
  (:require [clojure.spec.alpha :as s]))

(defn display-name
  {:t/sig '[(t/map {:opt {"name" (s/nilable string?)} :extra :open})
            :-> string?]}
  [{:strs [name] :or {"name" "Anonymous"}}]
  (or name "Unknown"))

(defn required-name
  {:t/sig '[(t/map {:opt {"name" string? "schema" nil?} :extra :open})
            :-> string?]}
  [{:strs! [name & "schema"]}]
  name)

(ns demo.query-string
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(defn fields
  {:t/sig '[string? java.util.regex.Pattern :-> (s/coll-of string? :kind vector?)]}
  [text pattern]
  (str/split text pattern))

(defn field-value
  {:t/sig '[string? :-> (s/nilable string?)]}
  [text]
  (let [[_ value] (str/split text (re-pattern #"=") 2)]
    value))

(defn normalized-value
  {:t/sig '[(t/map {:opt {:value (s/nilable string?)}}) :-> string?]}
  [request]
  (if (str/blank? (:value request))
    ""
    (str/replace (:value request) #"\+" " ")))

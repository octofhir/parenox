(ns demo.paths-bad
  (:require [clojure.string :as str]))

(defn nil-name
  {:t/sig '[:-> string?]}
  []
  (str/upper-case (get-in {:patient {:name nil}} [:patient :name] "Unknown")))

(defn wrong-parent
  {:t/sig '[:-> string?]}
  []
  (get-in (assoc-in {:patient 42} [:patient :name] "Ada") [:patient :name]))

(ns demo.location-helper
  (:require [clojure.test :refer [is]]))

(defn assert-value [value]
  (is (= :ready value) "shared assertion"))

(definterface LocalReader (readValue []))

(deftype BrokenReader []
  LocalReader
  (readValue [_]
    (let [text "short"]
      (.charAt ^String text 10))))

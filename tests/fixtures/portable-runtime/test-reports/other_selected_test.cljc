(ns portable.other-selected-test
  (:require [clojure.test :refer [deftest is]]))

(deftest fails
  (println "selected body failure")
  (is (= :expected :actual) "selected assertion mismatch"))

(deftest borrowed
  (println "unexpected referred test")
  (is false))

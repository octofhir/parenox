(ns demo.portable-report-test
  (:require [clojure.test :refer [deftest is]]))

(deftest passing-assertion
  (println "portable test output")
  (is (= 4 (+ 2 2))))

(deftest incorrect-result
  (is (= :expected :actual) "portable assertion mismatch"))

(deftest exception-in-assertion
  (is (= :expected (throw (ex-info "portable assertion exception" {:code :boom})))))

(deftest exception-in-test-body
  (throw (ex-info "portable test exception" {:code :boom})))

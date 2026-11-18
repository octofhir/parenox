(ns demo.focused-test
  (:require [clojure.test :refer [deftest deftest- is use-fixtures]]))

(def ^:dynamic *fixtures* #{})

(use-fixtures :once (fn [run]
                      (println "once fixture")
                      (binding [*fixtures* (conj *fixtures* :once)] (run))))

(use-fixtures :each (fn [run]
                      (binding [*fixtures* (conj *fixtures* :each)] (run))))

(deftest passes
  (is (= #{:once :each} *fixtures*)))

(deftest- private-pass
  (is (= #{:once :each} *fixtures*)))

(deftest fails
  (is (= :expected :actual)))

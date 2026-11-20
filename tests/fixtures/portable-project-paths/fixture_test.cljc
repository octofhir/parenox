(ns portable.fixture-test
  (:require [clojure.test :refer [deftest is]]
            [portable.choice :as choice]))

(deftest aliases-select-first
  (println "selected" choice/selected)
  (is (= :first choice/selected)))

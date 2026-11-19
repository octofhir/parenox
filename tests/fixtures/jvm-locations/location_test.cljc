(ns demo.location-test
  (:require [clojure.test :refer [deftest is]]
            [demo.location-helper :as helper]))

(deftest assertion-in-helper
  (helper/assert-value :pending))

(deftest error-in-type
  (is (= :ready (.readValue (helper/->BrokenReader)))))

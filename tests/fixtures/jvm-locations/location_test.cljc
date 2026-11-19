(ns demo.location-test
  (:require [clojure.test :refer [deftest is]]
            [demo.location-helper :as helper]))

(deftest assertion-in-helper
  (helper/assert-value :pending))

(deftest error-in-type
  (is (= :ready (.readValue (helper/->BrokenReader)))))

(deftest leaves-worker-running
  (let [started (promise)
        worker (Thread. (fn []
                          (deliver started true)
                          (Thread/sleep 10000)
                          (spit "worker-finished" "finished")))]
    (.setDaemon worker false)
    (.start worker)
    (is (deref started 10000 false))))

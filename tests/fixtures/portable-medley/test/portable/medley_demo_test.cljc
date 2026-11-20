(ns portable.medley-demo-test
  (:require [clojure.test :refer [deftest is]]
            [portable.medley-demo :as demo]))

(deftest same-medley-operations
  (let [[found merged remaining present] (demo/scenario)]
    (is (= 5 found))
    (is (= [1 2] merged))
    (is (= {:keep 9} remaining))
    (is (= [false 7 false] present))))

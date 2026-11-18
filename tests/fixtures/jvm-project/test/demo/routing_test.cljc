(ns demo.routing-test
  (:require [clojure.test :refer [deftest is testing]]
            [demo.routing :as routing]))

(deftest route-status
  (testing "a nil route is missing"
    (is (= :missing (routing/status {:route nil}))))
  (is (= :jvm (routing/platform))))

#?(:clj
   (deftest uses-project-options
     (is (= "from-alias" (System/getProperty "parenox.test.flag")))))

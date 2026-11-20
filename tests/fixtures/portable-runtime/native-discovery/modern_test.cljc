#?(:clj (ns discovery.jvm-only-test)
   :cljrs (ns discovery.modern-test
            (:require [clojure.test :refer [deftest is]]))
   :default (ns discovery.fallback-test))

(deftest selects-the-cljrs-branch
  (is (= :cljrs #?(:clj :jvm :cljrs :cljrs :default :fallback))))

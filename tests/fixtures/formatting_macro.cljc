(ns formatting.macro-example (:require [clojure.test]))

(clojure.test/testing  "a portable calculation 🐈"
    ;; Keep the dropped form next to the checks.
  #_(clojure.test/is    false)
  #?(:clj  (clojure.test/is  (= [1  2] [1 2]))
     :cljs  (clojure.test/is  (= [3  4] [3 4]))))

(def   unchanged  1)

(ns indexed (:require [clojure.spec.alpha :as s]))

(defn lookup {:t/sig '[(s/and int? #{0}) :-> #{:found :missing}]}
  [key] (get [:found] key :missing))
(defn lookup-generic {:t/sig '[(s/and integer? #{0}) :-> #{:found :missing}]}
  [key] (get [:found] key :missing))
(defn matches {:t/sig '[(s/and int? #{0}) :-> boolean?]}
  [key] (contains? [:found] key))
(defn string-lookup {:t/sig '[(s/and int? #{0}) :-> char?]}
  [key] (get "abc" key :missing))
(defn indexed-invocation {:t/sig '[(s/and int? #{0}) :-> #{:found :missing}]}
  [key] (try ([:found] key) (catch IllegalArgumentException _ :missing)))
(defn after-case {:t/sig '[int? :-> #{:found :missing}]}
  [key] (case key 0 (get [:found] key :missing) :found))
(defn after-min {:t/sig '[(s/and int? #{0}) :-> #{:found :missing}]}
  [key] (get [:found] (min key 1) :missing))
(defn through-path {:t/sig '[(s/and int? #{0}) :-> #{:found :missing}]}
  [key] (get-in [{:item :found}] [key :item] :missing))
(defn after-addition {:t/sig '[(s/and int? #{0}) :-> #{:found}]}
  [key] (get [:found] (+ key 0) :missing))

;; JVM-only assertions.
(doseq [[box expected present] [[byte :missing false] [short :missing false]
                                [int :found true] [long :found true]]
        :let [key (box 0)]]
  (assert (every? #(= expected (% key))
                  [lookup lookup-generic indexed-invocation after-case after-min through-path]))
  (assert (= present (matches key)))
  (assert (= \a (string-lookup key)))
  (assert (= :found (after-addition key))))
(assert (= :found (lookup-generic 0N)))
(println "Clojure/JVM indexed key classes: 17 assertions passed")

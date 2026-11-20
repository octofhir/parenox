(ns demo.signed-long-types (:require [clojure.spec.alpha :as s]))

(s/def ::fixed (s/and int? #{2147483648 -9223372036854775808 9223372036854775807}))
(s/def ::any-int (s/and integer? #{2147483648 -9223372036854775808 9223372036854775807}))
(s/def ::checked-int (s/and integer? (s/conformer int)))

;; JVM-only assertions. Run with clojure -M tests/fixtures/signed-long-types.clj

(defn outcome [f]
  (try [:value (f)]
       (catch ArithmeticException e [:arithmetic (.getMessage e)])
       (catch IllegalArgumentException _ [:illegal-argument])))

(doseq [n [2147483648 -2147483649 -9223372036854775808 9223372036854775807]]
  (assert (integer? n))
  (assert (int? n))
  (assert (= Long (class n)))
  (assert (= [:arithmetic "integer overflow"] (outcome #(s/conform ::checked-int n)))))

(doseq [n [2147483648N -2147483649N -9223372036854775808N 9223372036854775807N]]
  (assert (integer? n))
  (assert (not (int? n)))
  (assert (= clojure.lang.BigInt (class n)))
  (assert (= [:arithmetic "integer overflow"] (outcome #(s/conform ::checked-int n)))))

(doseq [n [9223372036854775808 9223372036854775808N -9223372036854775809N]]
  (assert (= clojure.lang.BigInt (class n)))
  (assert (= [:illegal-argument] (outcome #(s/conform ::checked-int n)))))

(doseq [[fixed wide] [[2147483648 2147483648N]
                      [-9223372036854775808 -9223372036854775808N]
                      [9223372036854775807 9223372036854775807N]]]
  (assert (= fixed wide))
  (assert (s/valid? ::fixed fixed))
  (assert (not (s/valid? ::fixed wide)))
  (assert (= ::s/invalid (s/conform ::fixed wide)))
  (assert (s/valid? ::any-int fixed))
  (assert (s/valid? ::any-int wide))
  (assert (= clojure.lang.BigInt (class (s/conform ::any-int wide)))))

(assert (= 2147483648 0x80000000 4294967296/2 (biginteger "2147483648")))
(doseq [other [2147483648.0 2147483648M 2147483649 2147483649N]]
  (assert (not (= 2147483648 other)))
  (assert (not (s/valid? ::any-int other))))

(doseq [[value expected] [[(inc 2147483647) 2147483648]
                          [(dec -2147483648) -2147483649]
                          [(apply dec [2147483648]) 2147483647]
                          [(inc -2147483649) -2147483648]
                          [(- 2147483648 2147483647) 1]
                          [(+ 9223372036854775806 1) 9223372036854775807]]]
  (assert (= expected value))
  (assert (= Long (class value))))
(assert (= Integer (class (s/conform ::checked-int (- 2147483648 2147483647)))))
(assert (= [:arithmetic "long overflow"] (outcome #(inc 9223372036854775807))))
(assert (= [:arithmetic "long overflow"] (outcome #(- -9223372036854775808))))

(assert (= [:arithmetic "integer overflow"] (outcome #(nth [:zero] 4294967296 :outside))))
(assert (= :zero ([:zero] 4294967296)))
(assert (= :zero ([:zero] -4294967296)))

(assert (= :wide (case 2147483648N 2147483648 :wide :miss)))
(assert (= :miss (case 2147483648 -2147483648 :small :miss)))

(prn :signed-long-types-ok (clojure-version))

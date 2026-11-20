(ns fixed-arithmetic)

;; JVM reference: clojure -M tests/fixtures/fixed-arithmetic.clj

(defn outcome [f]
  (try [:value (f)]
       (catch ArithmeticException e [:arithmetic (.getMessage e)])))

(doseq [[value expected]
        [[(+) 0]
         [(*) 1]
         [(+ 42) 42]
         [(* 42) 42]
         [(- 42) -42]
         [(+ 16r20 8 2) 42]
         [(clojure.core/- 50 3 5) 42]
         [(* 2 3 7) 42]
         [(inc (int 41)) 42]
         [(dec (byte 43)) 42]
         [(+ (short 20) (int 22)) 42]
         [(+ 2147483647 1) 2147483648]]]
  (assert (= expected value))
  (assert (instance? Long value)))

(let [boxed (Integer/valueOf 1000)]
  (assert (identical? boxed (+ boxed)))
  (assert (identical? boxed (* boxed)))
  (assert (every? #(instance? Long %) [(- boxed) (inc boxed) (dec boxed)])))

(assert (instance? Integer (int (+ 40 2))))
(assert (= [:arithmetic "integer overflow"] (outcome #(int (+ 2147483647 1)))))
(assert (instance? clojure.lang.BigInt (+ 1N 2)))
(assert (instance? Double (+ 1 1.0)))

;; An intermediate result can overflow even when the mathematical result fits.
(doseq [f [#(+ 9223372036854775807 1 -1)
           #(- -9223372036854775808 1 -1)
           #(* 9223372036854775807 2 0)
           #(- -9223372036854775808)
           #(inc 9223372036854775807)
           #(dec -9223372036854775808)]]
  (assert (= [:arithmetic "long overflow"] (outcome f))))

(let [order (atom [])
      mark (fn [tag value] (swap! order conj tag) value)]
  (assert (= [:arithmetic "long overflow"]
             (outcome #(+ (mark :first 9223372036854775807)
                          (mark :second 1)
                          (mark :third -1)))))
  (assert (= [:first :second] @order))
  (reset! order [])
  (assert (= [:arithmetic "long overflow"]
             (outcome #(apply + [(mark :first 9223372036854775807)
                                 (mark :second 1)
                                 (mark :third -1)]))))
  (assert (= [:first :second :third] @order)))

;; Apply the flag during compilation. An already compiled body stays checked.
(binding [*unchecked-math* true]
  (assert (= [:arithmetic "long overflow"] (outcome #(inc 9223372036854775807)))))
(doseq [mode [false true :warn-on-boxed]]
  (binding [*unchecked-math* mode]
    (assert (= [:value 42] (outcome #(eval '(+ 40 2)))))
    (assert (= (if mode [:value -9223372036854775808] [:arithmetic "long overflow"])
               (outcome #(eval '(inc 9223372036854775807)))))
    (assert (= (if mode [:value 9223372036854775807] [:arithmetic "long overflow"])
               (outcome #(eval '(+ 9223372036854775807 1 -1)))))
    (assert (= [:arithmetic "long overflow"]
               (outcome #(eval '(apply + [9223372036854775807 1])))))))

(prn {:clojure (clojure-version) :fixed-arithmetic :ok})

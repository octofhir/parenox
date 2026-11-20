(ns case-integers)

;; JVM reference: clojure -M tests/fixtures/case-integers.clj

(defn int-case [value]
  (case value
    (0x2a 84/3) :small
    43N :next
    :miss))

(defn mixed-case [value]
  (case value
    (84/2 2147483648N) :integer
    -2147483649 :negative
    :keyword :keyword
    :miss))

(doseq [value [42 (int 42) (byte 42) (short 42) 42N (biginteger 42) 84/2]]
  (assert (= :small (int-case value)))
  (assert (= :integer (mixed-case value))))

(doseq [index [1 (int 1) (byte 1) (short 1) 1N (biginteger 1) 0x1]]
  (assert (= :one (case index (0 1N) (nth [:zero :one] index) :miss))))

(assert (= :small (int-case 28N)))
(assert (= :next (int-case 43)))
(assert (= :keyword (mixed-case :keyword)))
(doseq [value [2147483648 2147483648N (biginteger 2147483648) 4294967296/2]]
  (assert (= :integer (mixed-case value)))
  (assert (= :miss (int-case value))))
(assert (= :negative (mixed-case -2147483649N)))
(assert (= :match (case 9223372036854775808/4611686018427387904 2 :match :miss)))
(assert (= :match (case 2 9223372036854775808/4611686018427387904 :match :miss)))

;; The low 32 bits of a wider integer are insufficient for a match.
(doseq [value [42.0 (float 42) 42M 85/2 4294967338 4294967338N 18446744073709551658N]]
  (assert (= :miss (int-case value)))
  (assert (= :miss (mixed-case value))))
(assert (= :miss (mixed-case 2147483648.0)))
(assert (= :miss (mixed-case 2147483648M)))
(let [dispatch (fn [value] (case value 42 :integer 42.0 :floating 42M :decimal))]
  (assert (= [:integer :floating :decimal] (mapv dispatch [42N 42.0 42M]))))

(doseq [form ['(case x 42 :a 42N :b)
              '(case x (42 0x2a) :duplicate)
              '(case x (42 84/2) :duplicate)
              '(case x 2147483648 :a 2147483648N :b)]]
  (let [error (try (macroexpand-1 form) nil (catch Throwable e e))
        cause (last (take-while some? (iterate ex-cause error)))]
    (assert (instance? IllegalArgumentException cause))
    (assert (re-find #"^Duplicate case test constant:" (ex-message cause)))))

(assert (= "No matching clause: 43"
           (try (case 43 42 :integer)
                (catch IllegalArgumentException e (ex-message e)))))

(prn {:clojure (clojure-version) :case-integers :ok})

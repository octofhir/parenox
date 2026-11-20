(ns spec-position
  (:require [clojure.spec.alpha :as s]))

;; JVM reference: clojure -M tests/fixtures/spec-position.clj
;; Exact declarations from lsp4clj 2.0.1, coercer.clj lines 31–33.
(s/def ::line (s/and integer? (s/conformer int)))
(s/def ::character (s/and integer? (s/conformer int)))
(s/def ::position (s/keys :req-un [::line ::character]))

(defn assert-invalid [spec value]
  (assert (= ::s/invalid (s/conform spec value)))
  (assert (false? (s/valid? spec value))))

(defn assert-throws [spec value expected]
  (doseq [f [s/conform s/valid?]]
    (assert (= expected (try (f spec value) nil (catch Exception error (class error)))))))

(doseq [value [nil false ::s/invalid "1" 1.0 1.0M 1/2 \a]]
  (assert-invalid ::line value)
  (assert-invalid ::character value)
  (assert-invalid ::position {:line value :character 0})
  (assert-invalid ::position {:line 0 :character value}))

(doseq [value [nil false ::s/invalid [] (java.util.HashMap. {:line 0 :character 1})]]
  (assert-invalid ::position value))

(let [extra (Object.)]
  (doseq [value [0 -1 -2147483648 2147483647 -2147483648N 2147483647N
                 42N (biginteger "0") (biginteger "2147483647") 84/2]]
    (doseq [spec [::line ::character]]
      (let [result (s/conform spec value)]
        (assert (= value result))
        (assert (instance? Integer result))
        (assert (true? (s/valid? spec value)))))
    (let [value-map {:line value :character value :extra extra}
          result (s/conform ::position value-map)]
      (assert (= value-map result))
      (assert (instance? Integer (:line result)))
      (assert (instance? Integer (:character result)))
      (assert (identical? extra (:extra result)))
      (assert (true? (s/valid? ::position value-map))))))

(doseq [[value expected] [[2147483648 ArithmeticException]
                          [-2147483649 ArithmeticException]
                          [2147483648N ArithmeticException]
                          [-2147483649N ArithmeticException]
                          [(biginteger "2147483648") ArithmeticException]
                          [9223372036854775808N IllegalArgumentException]
                          [-9223372036854775809N IllegalArgumentException]
                          [(biginteger "9223372036854775808") IllegalArgumentException]]]
  (assert-throws ::line value expected)
  (assert-throws ::character value expected)
  (assert-throws ::position {:line value :character 0} expected)
  (assert-throws ::position {:line 0 :character value} expected)
  (assert-invalid ::position {:line value})
  (assert-invalid ::position {:character value}))

;; Qualified registrations cannot satisfy the unqualified presence checks.
(assert-invalid ::position {::line 2147483648 ::character 1})
(let [input {:line 1 :character 2 ::line 3N ::character 4N}
      result (s/conform ::position input)]
  (assert (= input result))
  (assert (instance? Integer (::line result)))
  (assert (instance? Integer (::character result)))
  (assert (true? (s/valid? ::position input))))

;; After checking presence, s/keys walks the value map in its iteration order.
(assert-invalid ::position (array-map :line false :character 2147483648))
(assert-throws ::position (array-map :character 2147483648 :line false) ArithmeticException)
(assert-invalid ::position (array-map :character ::s/invalid :line 9223372036854775808N))
(assert-throws ::position (array-map :line 9223372036854775808N :character ::s/invalid)
               IllegalArgumentException)

;; integer? accepts BigInteger subclasses, and int calls virtual JVM methods.
(let [custom (proxy [java.math.BigInteger] ["2"]
               (bitLength [] (throw (IllegalStateException. "custom bitLength"))))]
  (assert (integer? custom))
  (assert-throws ::line custom IllegalStateException)
  (assert-throws ::position {:line custom :character 0} IllegalStateException))

(prn {:clojure (clojure-version) :spec-position :ok})

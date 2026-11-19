(ns lsp4clj.coercer (:require [clojure.spec.alpha :as s]))

;; lsp4clj v2.0.1, coercer.clj lines 17–18.
(s/def :error/code (s/and (s/or :kw keyword? :int int?)
                          (s/conformer second)))

;; lsp4clj v2.0.1, coercer.clj lines 31–37.
(s/def ::line (s/and integer? (s/conformer int)))
(s/def ::character (s/and integer? (s/conformer int)))
(s/def ::position (s/keys :req-un [::line ::character]))
(s/def ::start ::position)
(s/def ::end ::position)
(s/def ::range (s/keys :req-un [::start ::end]))
(s/def ::selection-range ::range)

;; JVM-only assertions: clojure -M tests/fixtures/spec-int-conformer.clj

(defn protocol-range [line]
  {:start {:line line :character 1} :end {:line 1 :character 2}})

(defn exception-class [f]
  (try (f) nil (catch Exception e (class e))))

(doseq [[value expected-class integral? fixed? conforms?]
        [[2147483648 Long true true true]
         [1N clojure.lang.BigInt true false false]
         [(biginteger 1) java.math.BigInteger true false false]
         [:failed clojure.lang.Keyword false false true]
         ["failed" String false false false]
         [1/2 clojure.lang.Ratio false false false]
         [2/2 Long true true true]
         [9223372036854775808/9223372036854775808 clojure.lang.BigInt true false false]
         [-9223372036854775808/1 clojure.lang.BigInt true false false]]]
  (assert (= expected-class (class value)))
  (assert (= integral? (integer? value)))
  (assert (= fixed? (int? value)))
  (assert (= (if conforms? value ::s/invalid) (s/conform :error/code value))))

(assert (= Integer (class (s/conform ::line 1N))))

(doseq [value [42 42N 84/2 -2147483648 2147483647]]
  (assert (s/valid? ::line value))
  (assert (= value (s/conform ::line value)))
  (assert (instance? Integer (s/conform ::line value)))
  (assert (s/valid? ::range (protocol-range value)))
  (assert (instance? Integer (:line (:start (s/conform ::range (protocol-range value)))))))

(doseq [value ["42" 42.0 42M 85/2]]
  (assert (not (s/valid? ::line value)))
  (assert (= ::s/invalid (s/conform ::line value)))
  (assert (not (s/valid? ::range (protocol-range value))))
  (assert (= ::s/invalid (s/conform ::range (protocol-range value)))))

(doseq [[value error] [[2147483648 ArithmeticException]
                       [-2147483649 ArithmeticException]
                       [2147483648N ArithmeticException]
                       [9223372036854775808N IllegalArgumentException]]]
  (assert (integer? value))
  (assert (= error (exception-class #(s/valid? ::line value))))
  (assert (= error (exception-class #(s/conform ::line value))))
  (assert (= error (exception-class #(s/valid? ::range (protocol-range value)))))
  (assert (= error (exception-class #(s/conform ::range (protocol-range value))))))

(let [int identity]
  (let [shadowed (s/and integer? (s/conformer int))]
    (assert (= 2147483648 (s/conform shadowed 2147483648)))))

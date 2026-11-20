(ns demo.error-code (:require [clojure.spec.alpha :as s]))

;; The tagged projection and required error fields use the lsp4clj 2.0.1 pattern.
(s/def :error/code (s/and (s/or :kw keyword? :int int?) (s/conformer second)))
(s/def :error/message string?)
(s/def ::error (s/keys :req-un [:error/code :error/message]))
(s/def ::errors (s/coll-of ::error))
(s/def ::code :error/code)
(s/def ::keyword keyword?)
(s/def ::reversed
  (s/and (s/or :fixed clojure.core/int? :keyword clojure.core/keyword?)
         (s/conformer clojure.core/second)))
(s/def ::keyword-only (s/and (s/or :kw keyword?) (s/conformer second)))

;; JVM-only assertions. Run with clojure -M tests/fixtures/spec-error-code.clj

(doseq [spec [:error/code ::code ::reversed]
        input [:ready :server/retry (byte 7) (short 1000) (int 100000) (long 1000000)
               2147483648 Long/MIN_VALUE Long/MAX_VALUE]]
  (assert (s/valid? spec input))
  (assert (identical? input (s/conform spec input))))

(doseq [spec [:error/code ::code ::reversed]
        input [::s/invalid nil false "ready" 'ready 7N (biginteger 7) 1.0 1M 3/2]]
  (assert (not (s/valid? spec input)))
  (assert (= ::s/invalid (s/conform spec input))))

(assert (= :ready (s/conform ::keyword-only :ready)))
(assert (= ::s/invalid (s/conform ::keyword-only 7)))

(let [left {:code :ready :message "prepared"}
      right {:code 2147483648 :message "wide code"}
      entry (clojure.lang.MapEntry/create left right)]
  (assert (identical? left (s/conform ::error left)))
  (assert (identical? right (s/conform ::error right)))
  (assert (identical? entry (s/conform ::errors entry)))
  (assert (= ::s/invalid (s/conform ::error (assoc right :code ::s/invalid))))
  (assert (= ::s/invalid (s/conform ::errors (clojure.lang.MapEntry/create left (assoc right :code 7N))))))

(s/def ::converting-branch
  (s/and (s/or :kw keyword? :int (s/and integer? (s/conformer int)))
         (s/conformer second)))
(let [input 1000
      output (s/conform ::converting-branch input)]
  (assert (= input output))
  (assert (= Integer (type output)))
  (assert (not (identical? input output))))

(prn :error-code-spec-ok (clojure-version))

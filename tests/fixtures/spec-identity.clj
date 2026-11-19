(ns demo.identity-spec (:require [clojure.spec.alpha :as s]))

(s/def ::message string?)
(s/def ::response-error (s/keys :req-un [::message]))
(s/def ::truth identity)
(s/def ::any-or-error
  (s/and (s/or :error ::response-error :any identity)
         (s/conformer second)))

;; JVM-only assertions: clojure -M tests/fixtures/spec-identity.clj

(doseq [value [nil false ::s/invalid]]
  (assert (not (s/valid? ::truth value)))
  (assert (= ::s/invalid (s/conform ::any-or-error value))))

(doseq [value [true 0 "" [] {} :ok {:message "error"}]]
  (assert (s/valid? ::truth value))
  (assert (= value (s/conform ::any-or-error value))))

(assert (identity ::s/invalid))
(assert (not (s/valid? any? ::s/invalid)))
(assert (s/valid? any? nil))
(assert (s/valid? any? false))

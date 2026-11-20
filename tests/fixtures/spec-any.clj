(ns demo.any-spec (:require [clojure.spec.alpha :as s]))

(s/def ::value any?)
(s/def ::values (s/coll-of any? :kind vector?))
(s/def ::entries (s/coll-of any? :kind map?))
(s/def ::collection (s/coll-of any?))

;; JVM-only assertions: clojure -M tests/fixtures/spec-any.clj

(doseq [value [nil false true 0 "" [] {} :ok [::s/invalid]]]
  (assert (s/valid? ::value value))
  (assert (= value (s/conform ::value value))))
(assert (any? ::s/invalid))
(assert (not (s/valid? ::value ::s/invalid)))
(assert (= ::s/invalid (s/conform ::value ::s/invalid)))

(assert (s/valid? ::values [nil false [::s/invalid]]))
(assert (not (s/valid? ::values [::s/invalid])))
(assert (not (s/valid? ::values '(nil false))))
(assert (s/valid? ::entries {::s/invalid ::s/invalid}))
(assert (not (s/valid? ::entries [:ok])))

(doseq [coll [[nil false] '(nil false) #{nil false} [[::s/invalid]]
              {::s/invalid ::s/invalid} (first {:a :b})]]
  (assert (s/valid? ::collection coll))
  (assert (= (type coll) (type (s/conform ::collection coll)))))

(doseq [coll [nil [::s/invalid] #{::s/invalid} (list ::s/invalid)
              (first {::s/invalid ::s/invalid}) "text" (to-array [false])
              (java.util.ArrayList.)]]
  (assert (not (s/valid? ::collection coll))))

;; The unkinded spec accepts queues and seqs, then conforms them as lists.
(doseq [coll [(conj clojure.lang.PersistentQueue/EMPTY false)
              (lazy-seq (list false))
              (seq (to-array [false]))]]
  (assert (s/valid? ::collection coll))
  (assert (list? (s/conform ::collection coll))))

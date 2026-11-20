(ns demo.qualified-keys
  (:require [clojure.spec.alpha :as s]))

(s/def ::line (s/and integer? (s/conformer int)))
(s/def ::kind keyword?)
(s/def ::label string?)
(s/def ::flag any?)
(s/def ::shape
  (s/keys :req [::line ::raw]
          :opt [::kind ::annotation]
          :req-un [::label]
          :opt-un [::flag]))
(s/def ::shapes (s/coll-of ::shape))
(s/def ::two-spellings (s/keys :req [::line] :opt-un [::line]))

;; JVM-only assertions.

(defn conform-result [spec input]
  (try (s/conform spec input)
       (catch ArithmeticException _ :overflow)))

(let [base {::line (int 1) ::raw nil :label "item"}
      optional (assoc base ::kind :region ::annotation ::s/invalid :flag false)
      unregistered (assoc optional ::raw ::s/invalid)]
  (doseq [value [base optional unregistered]]
    (assert (s/valid? ::shape value))
    (assert (identical? value (s/conform ::shape value))))
  (doseq [value [(-> base (dissoc ::line) (assoc :line 1))
                 (-> base (dissoc :label) (assoc ::label "item"))
                 (assoc base ::kind ::s/invalid)
                 (assoc base :flag ::s/invalid)]]
    (assert (not (s/valid? ::shape value)))
    (assert (= ::s/invalid (s/conform ::shape value))))
  (let [extra (assoc base :kind ::s/invalid)]
    (assert (identical? extra (s/conform ::shape extra))))
  (let [entry (clojure.lang.MapEntry/create base optional)
        changed (clojure.lang.MapEntry/create (assoc base ::line 1N) optional)
        result (s/conform ::shapes changed)]
    (assert (identical? entry (s/conform ::shapes entry)))
    (assert (= [base optional] result))
    (assert (vector? result))
    (assert (not (map-entry? result)))
    (assert (= Integer (type (::line (first result)))))
    (assert (identical? optional (second result))))
  (let [both (s/conform ::two-spellings {::line 1N :line 2N})]
    (assert (= {::line 1 :line 2} both))
    (assert (= [Integer Integer] (mapv type [(::line both) (:line both)]))))
  (assert (= ::s/invalid (s/conform ::two-spellings {::line 1N :line nil})))
  (assert (= ::s/invalid (s/conform ::two-spellings {:line 1N})))

  (let [foreign (assoc base :external/extra 7N)]
    (assert (identical? foreign (s/conform ::shape foreign)))
    (s/def :external/extra (s/and integer? (s/conformer int)))
    (assert (= Integer (type (:external/extra (s/conform ::shape foreign)))))
    (let [wide (assoc foreign :external/extra 2147483648)]
      (assert (= :overflow (conform-result ::shape wide)))
      (assert (= ::s/invalid (conform-result ::shape (dissoc wide ::line))))))

  (s/def ::raw keyword?)
  (assert (= ::s/invalid (s/conform ::shape base)))
  (assert (s/valid? ::shape (assoc base ::raw :value)))
  (s/def ::annotation any?)
  (assert (= ::s/invalid (s/conform ::shape (assoc unregistered ::raw :value)))))

(prn :qualified-keys-conformance-ok)

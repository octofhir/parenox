(ns demo.extra-registry
  (:require [clojure.spec.alpha :as s]))

(s/def ::label string?)
(s/def ::row (s/keys :req-un [::label]))
(s/def ::rows (s/coll-of ::row))
(s/def :external/line (s/and integer? (s/conformer int)))

;; JVM-only assertions.

(def ready-states {1 :ready 2 :failed})
(def retry-states {1 :retry 2 :failed})

(let [row {:label "result" :external/status 1}]
  (assert (identical? row (s/conform ::row row)))
  (s/def :external/status (s/and int? ready-states (s/conformer ready-states)))
  (assert (s/valid? ::row row))
  (assert (= :ready (:external/status (s/conform ::row row))))
  (doseq [invalid [3 1N]]
    (let [input (assoc row :external/status invalid)]
      (assert (not (s/valid? ::row input)))
      (assert (= ::s/invalid (s/conform ::row input)))))
  (let [result (s/conform ::rows (clojure.lang.MapEntry/create row row))]
    (assert (vector? result))
    (assert (not (map-entry? result)))
    (assert (= [:ready :ready] (mapv :external/status result))))
  (s/def :external/status (s/and int? retry-states (s/conformer retry-states)))
  (assert (= :retry (:external/status (s/conform ::row row))))
  (s/def :external/status nil)
  (assert (identical? row (s/conform ::row row))))

(let [boxed {:label "coordinate" :external/line (int 7)}
      boxed-entry (clojure.lang.MapEntry/create boxed boxed)
      unboxed (assoc boxed :external/line 7N)
      result (s/conform ::rows (clojure.lang.MapEntry/create boxed unboxed))]
  (assert (identical? boxed-entry (s/conform ::rows boxed-entry)))
  (assert (vector? result))
  (assert (not (map-entry? result)))
  (assert (= [boxed boxed] result))
  (assert (identical? boxed (first result)))
  (assert (= Integer (type (:external/line (second result))))))

(let [wide {:label "coordinate" :external/line 2147483648}]
  (assert (= :overflow
             (try (s/conform ::row wide)
                  (catch ArithmeticException _ :overflow))))
  (assert (= ::s/invalid (s/conform ::row (dissoc wide :label)))))

(s/def :external/opaque (s/or :text string?))
(assert (= {:label "result" :external/opaque [:text "text"]}
           (s/conform ::row {:label "result" :external/opaque "text"})))

(prn :extra-registry-conformance-ok)

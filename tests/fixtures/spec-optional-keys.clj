(ns demo.optional-keys
  (:require [clojure.spec.alpha :as s]))

(s/def ::optional-any any?)
(s/def ::optional-keyword keyword?)
(s/def ::optional-int (s/and integer? (s/conformer int)))
(s/def ::shape
  (s/keys :req-un [::required-unknown]
          :opt-un [::optional-unknown ::optional-any ::optional-keyword ::optional-int]))
(s/def ::shapes (s/coll-of ::shape))

;; JVM-only assertions.

(defn conform-result [spec input]
  (try (s/conform spec input)
       (catch ArithmeticException _ :overflow)))

(let [base {:required-unknown ::s/invalid}
      good (assoc base :optional-unknown ::s/invalid :optional-any nil :optional-keyword :region)
      with-int (assoc good :optional-int 7N)
      overflowing (assoc good :optional-int 2147483648)
      conformed (conform-result ::shape with-int)]
  (assert (= ::s/invalid (conform-result ::shape {:optional-int 2147483648})))
  (assert (identical? base (conform-result ::shape base)))
  (assert (identical? good (conform-result ::shape good)))
  (assert (= with-int conformed))
  (assert (= Integer (type (:optional-int conformed))))
  (assert (identical? conformed (conform-result ::shape conformed)))
  (doseq [value [nil false ::s/invalid]]
    (let [pass {:required-unknown value :optional-unknown value}]
      (assert (identical? pass (conform-result ::shape pass)))))
  (doseq [kv [[:optional-keyword nil] [:optional-keyword false] [:optional-keyword ::s/invalid]
              [:optional-any ::s/invalid] [:optional-int nil] [:optional-int 1.0]]]
    (assert (= ::s/invalid (conform-result ::shape (apply assoc base kv)))))
  (let [ordered-a (array-map :optional-int 2147483648 :optional-keyword nil :required-unknown false)
        ordered-b (array-map :optional-keyword nil :optional-int 2147483648 :required-unknown false)]
    (assert (= :overflow (conform-result ::shape ordered-a)))
    (assert (= ::s/invalid (conform-result ::shape ordered-b))))
  (assert (= ::s/invalid (conform-result ::shapes (list {} overflowing))))
  (assert (= :overflow (conform-result ::shapes (list overflowing {}))))
  (let [entry-ok (clojure.lang.MapEntry/create good conformed)
        entry-changed (clojure.lang.MapEntry/create good with-int)
        entry-invalid-first (clojure.lang.MapEntry/create {} overflowing)
        entry-overflow-first (clojure.lang.MapEntry/create overflowing {})
        changed (conform-result ::shapes entry-changed)
        as-set (conform-result ::shapes #{base with-int})]
    (assert (identical? entry-ok (conform-result ::shapes entry-ok)))
    (assert (vector? changed))
    (assert (not (map-entry? changed)))
    (assert (identical? good (first changed)))
    (assert (= Integer (type (:optional-int (second changed)))))
    (assert (set? as-set))
    (assert (= #{base conformed} as-set))
    (assert (some #(= Integer (type (:optional-int %))) as-set))
    (assert (= ::s/invalid (conform-result ::shapes entry-invalid-first)))
    (assert (= :overflow (conform-result ::shapes entry-overflow-first)))))

(let [input {:required-unknown false :foreign/other 1N :foreign/later ::s/invalid}]
  (assert (identical? input (conform-result ::shape input)))
  (s/def :foreign/other (s/and integer? (s/conformer int)))
  (let [converted (conform-result ::shape input)]
    (assert (= input converted))
    (assert (= Integer (type (:foreign/other converted))))
    (assert (= ::s/invalid (:foreign/later converted))))
  (assert (= ::s/invalid (conform-result ::shape (assoc input :foreign/other false))))
  (s/def :foreign/later any?)
  (assert (= ::s/invalid (conform-result ::shape input)))
  (s/def :foreign/later nil)
  (assert (map? (conform-result ::shape input)))
  (s/def ::optional-unknown any?)
  (assert (map? (conform-result ::shape input)))
  (assert (= ::s/invalid (conform-result ::shape (assoc input :optional-unknown ::s/invalid))))
  (s/def ::required-unknown keyword?)
  (assert (= ::s/invalid (conform-result ::shape input)))
  (assert (map? (conform-result ::shape (assoc input :required-unknown :new)))))

(prn :optional-keys-conformance-ok)

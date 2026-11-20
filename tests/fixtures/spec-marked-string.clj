(ns demo.marked-string
  (:require [clojure.spec.alpha :as s]))

;; The inline branch from lsp4clj 2.0.1.
(s/def ::marked-string
  (s/and (s/or :string string?
               :marked-string (s/keys :req-un [::language ::value]))
         (s/conformer second)))

;; JVM-only assertions.
(s/def ::marked-strings-test (s/coll-of ::marked-string))

(let [text (String. "code")
      marked {:language nil :value ::s/invalid :extra/unregistered ::s/invalid}
      entry (clojure.lang.MapEntry/create text marked)]
  (assert (s/valid? ::marked-string text))
  (assert (identical? text (s/conform ::marked-string text)))
  (assert (identical? marked (s/conform ::marked-string marked)))
  (assert (identical? entry (s/conform ::marked-strings-test entry)))

  ;; The required keys originally have no value specs.
  (doseq [value [nil false ::s/invalid]]
    (let [input {:language value :value value}]
      (assert (s/valid? ::marked-string input))
      (assert (identical? input (s/conform ::marked-string input)))))

  (doseq [input [nil false ::s/invalid :code 1 [] {} {:language nil} {:value false}]]
    (assert (not (s/valid? ::marked-string input)))
    (assert (= ::s/invalid (s/conform ::marked-string input))))

  (s/def :extra/line (s/and integer? (s/conformer int)))
  (let [input (assoc marked :extra/line 123N)
        converted (s/conform ::marked-string input)
        result (s/conform ::marked-strings-test
                          (clojure.lang.MapEntry/create text input))]
    (assert (= input converted))
    (assert (not (identical? input converted)))
    (assert (= Integer (type (:extra/line converted))))
    (assert (= [text converted] result))
    (assert (vector? result))
    (assert (not (map-entry? result)))
    (assert (identical? text (first result)))
    (assert (= Integer (type (:extra/line (second result))))))
  (assert (= ::s/invalid (s/conform ::marked-string {:extra/line 2147483648})))
  (assert (try
            (s/conform ::marked-string (assoc marked :extra/line 2147483648))
            false
            (catch ArithmeticException _ true)))

  ;; The existing inline map spec reads field registrations on each call.
  (s/def ::language keyword?)
  (assert (= ::s/invalid (s/conform ::marked-string marked)))
  (let [input {:language :clojure :value ::s/invalid}]
    (assert (identical? input (s/conform ::marked-string input)))
    (s/def ::value string?)
    (assert (= ::s/invalid (s/conform ::marked-string input))))
  (let [input {:language :clojure :value text}]
    (assert (identical? input (s/conform ::marked-string input)))))

(prn :marked-string-conformance-ok (clojure-version))

(ns demo.location-collection
  (:require [clojure.spec.alpha :as s]))

;; The location specs from lsp4clj 2.0.1, with no dependency on the library.
(s/def ::line (s/and integer? (s/conformer int)))
(s/def ::character (s/and integer? (s/conformer int)))
(s/def ::position (s/keys :req-un [::line ::character]))
(s/def ::start ::position)
(s/def ::end ::position)
(s/def ::range (s/keys :req-un [::start ::end]))
(s/def ::uri string?)
(s/def ::location (s/keys :req-un [::uri ::range]))
(s/def ::locations (s/coll-of ::location))

;; JVM-only assertions: clojure -M tests/fixtures/spec-location-collection.clj

(def location
  {:uri "file:///home.clj"
   :range {:start {:line 1N :character 2}
           :end {:line 3 :character 4}}})

(doseq [empty-coll [[] () #{} {}]]
  (assert (s/valid? ::locations empty-coll))
  (assert (= empty-coll (s/conform ::locations empty-coll)))
  (assert (= (type empty-coll) (type (s/conform ::locations empty-coll)))))
(assert (s/valid? ::locations clojure.lang.PersistentQueue/EMPTY))
(assert (list? (s/conform ::locations clojure.lang.PersistentQueue/EMPTY)))

(doseq [[locations result-kind] [[[location] vector?]
                                 [(list location) list?]
                                 [#{location} set?]
                                 [(conj clojure.lang.PersistentQueue/EMPTY location) list?]
                                 [(lazy-seq (list location)) list?]]]
  (assert (s/valid? ::locations locations))
  (let [result (s/conform ::locations locations)]
    (assert (result-kind result))
    (assert (= (seq locations) (seq result)))
    (assert (= java.lang.Integer (type (get-in (first result) [:range :start :line]))))))

(let [entry (first {location location})
      result (s/conform ::locations entry)]
  (assert (s/valid? ::locations entry))
  (assert (vector? result))
  (assert (not (map-entry? result)))
  (assert (= [location location] result))
  (assert (every? #(= java.lang.Integer (type (get-in % [:range :start :line]))) result)))

(let [converted (s/conform ::location location)
      entry (first {converted converted})]
  (assert (identical? entry (s/conform ::locations entry))))

(doseq [boundary [-128 127]]
  (let [converted (s/conform ::location (assoc-in location [:range :start :line] boundary))
        entry (clojure.lang.MapEntry/create converted converted)]
    (assert (identical? entry (s/conform ::locations entry)))))

(let [converted (s/conform ::location (assoc-in location [:range :start :line] 128))
      entry (clojure.lang.MapEntry/create converted converted)
      boxed (get-in converted [:range :start :line])
      same-box? (identical? boxed ((deref #'int) boxed))]
  ;; JVMs may cache additional values beyond the guaranteed range.
  (assert (= same-box? (map-entry? (s/conform ::locations entry)))))

(doseq [invalid [nil "text" ::s/invalid [::s/invalid] [nil]
                 {:entry location} (first {:entry location})
                 [(assoc location :uri ::s/invalid)]
                 [{:uri "file:///no-range.clj"}]]]
  (assert (not (s/valid? ::locations invalid)))
  (assert (= ::s/invalid (s/conform ::locations invalid))))

;; Named keys are checked even when they do not occur in :req-un.
(s/def ::extra string?)
(assert (= ::s/invalid (s/conform ::locations [(assoc location ::extra false)])))
(assert (= ::s/invalid (:annotation (first (s/conform ::locations [(assoc location :annotation ::s/invalid)])))))

;; A child conformer can throw during either validation or conformance.
(let [wide [(assoc-in location [:range :end :line] 2147483648)]]
  (assert (= ::s/invalid (s/conform ::locations (into [nil] wide))))
  (assert (= ::s/invalid (s/conform ::locations (clojure.lang.MapEntry/create nil (first wide)))))
  (assert (try
            (s/conform ::locations (clojure.lang.MapEntry/create location (first wide)))
            false
            (catch ArithmeticException _ true)))
  (doseq [operation [s/valid? s/conform]]
    (assert (try
              (operation ::locations wide)
              false
              (catch ArithmeticException _ true)))))

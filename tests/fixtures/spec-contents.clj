(ns demo.contents
  (:require [clojure.spec.alpha :as s]))

;; The contents specs from lsp4clj 2.0.1.
(s/def ::marked-string
  (s/and (s/or :string string?
               :marked-string (s/keys :req-un [::language ::value]))
         (s/conformer second)))

(s/def :markup/kind #{"plaintext" "markdown"})
(s/def :markup/value string?)
(s/def ::markup-content (s/keys :req-un [:markup/kind :markup/value]))

(s/def ::contents
  (s/and (s/or :marked-strings (s/coll-of ::marked-string)
               :markup-content ::markup-content)
         (s/conformer second)))

;; JVM-only assertions.
(def text (String. "source"))
(def marked {:language nil :value ::s/invalid})
(def markup {:kind (String. "markdown") :value (String. "contents")})

(defn conform-or-overflow [input]
  (try
    (s/conform ::contents input)
    (catch ArithmeticException _ :overflow)))

(assert (identical? markup (s/conform ::contents markup)))
(doseq [input [[] () #{} {}]]
  (let [result (s/conform ::contents input)]
    (assert (= input result))
    (assert (= (type input) (type result)))))

(let [mixed [text marked]
      entry (clojure.lang.MapEntry/create text marked)
      queue (conj clojure.lang.PersistentQueue/EMPTY text marked)
      hash-set #{text marked}]
  (assert (identical? mixed (s/conform ::contents mixed)))
  (assert (identical? entry (s/conform ::contents entry)))
  (doseq [input [(list text marked) (seq mixed) queue]]
    (let [result (s/conform ::contents input)]
      (assert (list? result))
      (assert (= mixed result))
      (assert (not (identical? input result)))
      (assert (identical? text (first result)))
      (assert (identical? marked (second result)))))
  (let [result (s/conform ::contents hash-set)]
    (assert (= (type hash-set) (type result)))
    (assert (= hash-set result))
    (assert (not (identical? hash-set result)))
    (assert (identical? text (get result text)))
    (assert (identical? marked (get result marked)))))

(doseq [input [nil false ::s/invalid text :unknown 0 [nil] [::s/invalid] [markup]
               marked {:entry marked} {:kind "html" :value "x"} {:kind "markdown"}
               (clojure.lang.MapEntry/create text markup)]]
  (assert (not (s/valid? ::contents input)))
  (assert (= ::s/invalid (s/conform ::contents input))))

(s/def :extra/line (s/and integer? (s/conformer int)))
(let [input (assoc marked :extra/line 7N)
      overflowing (assoc input :extra/line 2147483648)
      vector-input [text input]
      entry-input (clojure.lang.MapEntry/create text input)]
  (doseq [collection [vector-input entry-input]]
    (let [result (s/conform ::contents collection)]
      (assert (= vector-input result))
      (assert (vector? result))
      (assert (not (map-entry? result)))
      (assert (not (identical? collection result)))
      (assert (identical? text (first result)))
      (assert (= Integer (type (:extra/line (second result)))))))
  (assert (= Integer (type (:extra/line (first (s/conform ::contents #{input}))))))
  (assert (= Integer (type (:extra/line (s/conform ::contents (assoc markup :extra/line 7N))))))
  (doseq [ordered [vector list (fn [left right] (clojure.lang.MapEntry/create left right))]]
    (assert (= ::s/invalid (conform-or-overflow (ordered nil overflowing))))
    (assert (= :overflow (conform-or-overflow (ordered overflowing nil)))))
  (assert (= :overflow (conform-or-overflow (assoc markup :extra/line 2147483648))))
  (assert (= ::s/invalid (conform-or-overflow {:extra/line 2147483648})))
  (assert (= #{::s/invalid :overflow}
             (into #{} (for [salt (range 64)]
                         (conform-or-overflow #{{:tag salt} overflowing}))))))

(let [by-bigint-line (fn [left right]
                       (let [a (:extra/line left) b (:extra/line right)]
                         (when-not (and (instance? clojure.lang.BigInt a)
                                        (instance? clojure.lang.BigInt b))
                           (throw (ex-info "Expected unconverted coordinates" {:reason :converted})))
                         (compare a b)))
      input (sorted-set-by by-bigint-line (assoc marked :extra/line 7N)
                            (assoc marked :extra/line 8N))]
  (assert (= :converted
             (try
               (s/conform ::contents input)
               (catch clojure.lang.ExceptionInfo error
                 (:reason (ex-data error)))))))

(defn missing-spec? [attempt name]
  (try
    (attempt)
    false
    (catch Exception error
      (= (str "Unable to resolve spec: " name) (.getMessage error)))))

(s/def ::unresolved-choice
  (s/and (s/or :marked-strings (s/coll-of ::marked-string) :markup ::unregistered-markup)
         (s/conformer second)))
(assert (missing-spec? #(s/conform ::unresolved-choice []) ::unregistered-markup))

(s/def ::unresolved-element
  (s/and (s/or :marked-strings (s/coll-of ::unregistered-element) :markup ::markup-content)
         (s/conformer second)))
(doseq [input [[] markup text]]
  (assert (missing-spec? #(s/conform ::unresolved-element input) ::unregistered-element)))

(s/def ::markup-first
  (s/and (s/or :markup ::markup-content :marked-strings (s/coll-of ::unregistered-element))
         (s/conformer second)))
(assert (identical? markup (s/conform ::markup-first markup)))
(assert (missing-spec? #(s/conform ::markup-first []) ::unregistered-element))

(s/def ::language keyword?)
(s/def ::value string?)
(assert (= ::s/invalid (s/conform ::contents [text marked])))
(let [input [text {:language :clojure :value "contents"}]]
  (assert (identical? input (s/conform ::contents input))))

(prn :contents-conformance-ok (clojure-version))

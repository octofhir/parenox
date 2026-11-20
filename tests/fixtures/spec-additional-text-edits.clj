(ns spec-additional-text-edits
  (:require [clojure.spec.alpha :as s]))

;; JVM reference: clojure -M tests/fixtures/spec-additional-text-edits.clj
;; Exact declarations from lsp4clj 2.0.1, coercer.clj lines 31–36 and 63–65.
(s/def ::line (s/and integer? (s/conformer int)))
(s/def ::character (s/and integer? (s/conformer int)))
(s/def ::position (s/keys :req-un [::line ::character]))
(s/def ::start ::position)
(s/def ::end ::position)
(s/def ::range (s/keys :req-un [::start ::end]))
(s/def ::new-text string?)
(s/def ::text-edit (s/keys :req-un [::new-text ::range]))
(s/def ::additional-text-edits (s/coll-of ::text-edit))

;; Collection assertions.
(def edits ::additional-text-edits)
(def edit {:new-text (String. "🧪")
           :range {:start {:line 1N :character 2}
                   :end {:line 3 :character 4}}})
(def other (assoc edit :new-text "second"))
(defn at-line [value] (assoc-in edit [:range :start :line] value))
(def invalid (assoc edit :new-text nil))
(def overflow (at-line 2147483648))
(def beyond-long (at-line 9223372036854775808N))

(defn outcome [operation spec input]
  (try (operation spec input) (catch Exception error error)))

(defn assert-invalid [input]
  (assert (= ::s/invalid (s/conform edits input)))
  (assert (false? (s/valid? edits input))))

(defn assert-throws [input expected]
  (doseq [operation [s/conform s/valid?]]
    (assert (= expected (class (outcome operation edits input))))))

(defn assert-boxed [value]
  (doseq [side [:start :end]
          coordinate [:line :character]]
    (assert (instance? Integer (get-in value [:range side coordinate])))))

(assert (= (list 'clojure.spec.alpha/coll-of ::text-edit) (s/form edits)))
(doseq [empty-input [[] () #{} {}]]
  (let [result (s/conform edits empty-input)]
    (assert (= empty-input result))
    (assert (= (class empty-input) (class result)))
    (assert (true? (s/valid? edits empty-input)))))
(assert (list? (s/conform edits clojure.lang.PersistentQueue/EMPTY)))

(doseq [[input kind] [[[edit other] vector?]
                      [(list edit other) list?]
                      [(seq [edit other]) list?]
                      [(lazy-seq (list edit other)) list?]
                      [(conj clojure.lang.PersistentQueue/EMPTY edit other) list?]]]
  (let [result (s/conform edits input)]
    (assert (kind result))
    (assert (= [edit other] (vec result)))
    (assert (identical? (:new-text edit) (:new-text (first result))))
    (run! assert-boxed result)
    (assert (true? (s/valid? edits input)))))

(let [result (s/conform edits #{edit other})]
  (assert (set? result))
  (assert (= #{edit other} result))
  (run! assert-boxed result)
  (assert (true? (s/valid? edits #{edit other}))))

(let [entry (clojure.lang.MapEntry/create edit other)
      converted (s/conform edits entry)
      boxed-entry (clojure.lang.MapEntry/create (first converted) (second converted))]
  (assert (true? (s/valid? edits entry)))
  (assert (= [edit other] converted))
  (assert (vector? converted))
  (assert (not (map-entry? converted)))
  (run! assert-boxed converted)
  (assert (identical? boxed-entry (s/conform edits boxed-entry))))

;; A nonempty map supplies map entries as elements, not edit maps.
(doseq [value [nil false ::s/invalid "text" (java.util.ArrayList. [edit])
               [nil] [false] [::s/invalid] [invalid]
               {:edit edit} {edit other} {overflow edit}]]
  (assert-invalid value))
(assert-throws [overflow nil ::s/invalid] ArithmeticException)

(doseq [[value error] [[overflow ArithmeticException]
                       [beyond-long IllegalArgumentException]]
        make-coll [vector list #(seq [%1 %2]) #(clojure.lang.MapEntry/create %1 %2)]]
  (assert-invalid (make-coll invalid value))
  (assert-throws (make-coll value invalid) error))
(doseq [[value error] [[overflow ArithmeticException] [beyond-long IllegalArgumentException]]]
  (let [by-order #(compare (:order %1) (:order %2))]
    (assert-invalid (sorted-set-by by-order (assoc invalid :order 0) (assoc value :order 1)))
    (assert-throws (sorted-set-by by-order (assoc value :order 0) (assoc invalid :order 1)) error)))

(assert-invalid [(dissoc overflow :new-text)])
(assert-invalid [(update-in beyond-long [:range :start] dissoc :character)])

;; coll-of performs all conversions even when every would sample.
(binding [s/*coll-check-limit* 1]
  (assert (s/valid? (s/every ::text-edit) [edit invalid]))
  (assert-invalid [edit invalid])
  (assert-throws [edit overflow] ArithmeticException)
  (run! assert-boxed (s/conform edits [edit other])))
(assert-invalid (conj (vec (repeat 130 edit)) invalid))

;; The element spec is forced once. The s/keys checks within it remain live.
(s/def ::text-edit string?)
(assert (true? (s/valid? ::text-edit "new registration")))
(assert-invalid ["new registration"])
(assert (true? (s/valid? edits [edit])))
(s/def ::new-text boolean?)
(assert-invalid [edit])
(assert (= [(assoc edit :new-text false)]
           (s/conform edits [(assoc edit :new-text false)])))
(s/def ::new-text string?)
(s/def ::text-edit (s/keys :req-un [::new-text ::range]))

;; A first lookup before registration is cached as an exception.
(let [late (s/coll-of ::future-edit)
      failure (outcome s/conform late nil)]
  (assert (instance? Exception failure))
  (assert (= (str "Unable to resolve spec: " ::future-edit) (ex-message failure)))
  (s/def ::future-edit ::text-edit)
  (assert (identical? failure (outcome s/valid? late [edit])))
  (assert (true? (s/valid? (s/coll-of ::future-edit) [edit])))
  (s/def ::future-edit nil))

(prn {:clojure (clojure-version) :spec-additional-text-edits :ok})

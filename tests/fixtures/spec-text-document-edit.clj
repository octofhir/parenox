(ns spec-text-document-edit
  (:require [clojure.spec.alpha :as s]))

;; JVM reference: clojure -M tests/fixtures/spec-text-document-edit.clj
;; Exact declarations from lsp4clj 2.0.1 coercer.clj.
(s/def ::line (s/and integer? (s/conformer int)))
(s/def ::character (s/and integer? (s/conformer int)))
(s/def ::position (s/keys :req-un [::line ::character]))
(s/def ::start ::position)
(s/def ::end ::position)
(s/def ::range (s/keys :req-un [::start ::end]))
(s/def ::new-text string?)
(s/def ::text-edit (s/keys :req-un [::new-text ::range]))
(s/def ::version (s/and integer? (s/conformer int)))
(s/def ::uri string?)

(s/def ::edits (s/coll-of ::text-edit))
(s/def ::text-document (s/keys :req-un [::version ::uri]))
(s/def ::text-document-edit (s/keys :req-un [::text-document ::edits]))

;; Document edit assertions.
(def document {:version (biginteger "7") :uri (String. "file:///fixture")})
(def edit
  {:new-text (String. "🧪")
   :range {:start {:line 1N :character 2} :end {:line 3 :character 4N}}})
(def input {:text-document document :edits [edit]})
(def wide-version (assoc document :version 2147483648))
(def wide-edit (assoc-in edit [:range :end :line] 2147483648))
(def beyond-long (assoc-in edit [:range :start :line] 9223372036854775808N))

(defn assert-invalid [value]
  (assert (= ::s/invalid (s/conform ::text-document-edit value)))
  (assert (false? (s/valid? ::text-document-edit value))))

(defn assert-throws [value expected]
  (doseq [operation [s/conform s/valid?]]
    (assert (= expected (try
                          (operation ::text-document-edit value)
                          nil
                          (catch Exception error (class error)))))))

(defn assert-int-fields [value]
  (assert (instance? Integer (get-in value [:text-document :version])))
  (doseq [current (:edits value)
          side [:start :end]
          coordinate [:line :character]]
    (assert (instance? Integer (get-in current [:range side coordinate])))))

(let [empty-edits (with-meta [] {:fixture :empty})
      empty-input {:text-document (assoc document :version (int 7)) :edits empty-edits}
      result (s/conform ::text-document-edit empty-input)]
  (assert (identical? empty-input result))
  (assert (identical? empty-edits (:edits result)))
  (assert (true? (s/valid? ::text-document-edit empty-input))))

(doseq [make-map [array-map hash-map sorted-map]]
  (let [doc (with-meta (apply make-map (mapcat identity document)) {:fixture :document})
        range-map (with-meta (:range edit) {:fixture :range})
        first-edit (with-meta (assoc edit :range range-map) {:fixture :edit})
        edits (with-meta [first-edit (assoc first-edit :new-text "second")] {:fixture :edits})
        original (with-meta (make-map :text-document doc :edits edits) {:fixture :outer})
        result (s/conform ::text-document-edit original)]
    (assert (= original result))
    (assert-int-fields result)
    (doseq [path [[] [:text-document] [:edits] [:edits 0] [:edits 0 :range]]]
      (let [before (get-in original path)
            after (get-in result path)]
        (assert (= (class before) (class after)))
        (assert (= (meta before) (meta after)))))
    (assert (identical? (:uri document) (get-in result [:text-document :uri])))
    (assert (identical? (:new-text edit) (get-in result [:edits 0 :new-text])))
    (assert (true? (s/valid? ::text-document-edit original)))))

(defrecord TextDocument [version uri])
(defrecord DocumentEdit [text-document edits])
(let [original (with-meta (->DocumentEdit (map->TextDocument document) [edit])
                 {:fixture :record})
      result (s/conform ::text-document-edit original)]
  (assert (= original result))
  (assert (instance? DocumentEdit result))
  (assert (instance? TextDocument (:text-document result)))
  (assert (= (meta original) (meta result)))
  (assert-int-fields result)
  (assert (true? (s/valid? ::text-document-edit original))))

(doseq [value [nil false ::s/invalid]]
  (assert-invalid value)
  (assert-invalid (assoc input :text-document value))
  (assert-invalid (assoc input :edits value))
  (assert-invalid (assoc-in input [:text-document :version] value))
  (assert-invalid (assoc-in input [:text-document :uri] value))
  (assert-invalid (assoc input :edits [value])))
(assert-invalid (assoc input :edits [[]]))

;; Presence is checked before any values at each map level.
(assert-invalid {:text-document wide-version})
(assert-invalid {:edits [beyond-long]})
(assert-invalid (assoc input :text-document (dissoc wide-version :uri)))
(assert-invalid (assoc input :edits [(dissoc wide-edit :new-text)]))

;; When all required keys are present, the first failure encountered wins.
(assert-invalid (array-map :edits [false] :text-document wide-version))
(assert-throws (array-map :text-document wide-version :edits [false]) ArithmeticException)
(assert-throws (array-map :text-document wide-version :edits [beyond-long]) ArithmeticException)
(assert-throws (array-map :edits [beyond-long] :text-document wide-version) IllegalArgumentException)
(assert-invalid (assoc input :text-document (array-map :uri false :version 2147483648)))
(assert-throws (assoc input :text-document (array-map :version 2147483648 :uri false))
               ArithmeticException)
(assert-invalid (assoc input :edits [false wide-edit]))
(assert-throws (assoc input :edits [wide-edit false]) ArithmeticException)
(assert-invalid (assoc input :edits [::s/invalid beyond-long]))
(assert-throws (assoc input :edits [beyond-long ::s/invalid]) IllegalArgumentException)

(prn {:clojure (clojure-version) :spec-text-document-edit :ok})

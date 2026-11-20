(ns demo.text-edit
  (:require [clojure.spec.alpha :as s]))

;; The relevant lsp4clj 2.0.1 declarations, runnable without the library.
(s/def ::line (s/and integer? (s/conformer int)))
(s/def ::character (s/and integer? (s/conformer int)))
(s/def ::position (s/keys :req-un [::line ::character]))
(s/def ::start ::position)
(s/def ::end ::position)
(s/def ::range (s/keys :req-un [::start ::end]))
(s/def ::new-text string?)
(s/def ::text-edit (s/keys :req-un [::new-text ::range]))
(s/def ::additional-text-edits (s/coll-of ::text-edit))
(s/def ::edits (s/coll-of ::text-edit))
(s/def ::text-edits (s/coll-of ::text-edit))

;; JVM-only assertions.
(def edit
  {:new-text "🧪\n"
   :range {:start {:line 1N :character 2}
           :end {:line 3 :character 4}}})

(def second-edit
  (-> edit
      (assoc :new-text "second")
      (assoc-in [:range :start :line] 5N)))

(def overflow-edit
  (assoc-in edit [:range :start :line] 2147483648))

(defn int-coordinates? [text-edit]
  (every? #(= Integer (type %))
          (for [side [:start :end]
                coordinate [:line :character]]
            (get-in text-edit [:range side coordinate]))))

(defn conform-or-overflow [spec input]
  (try
    (s/conform spec input)
    (catch ArithmeticException _ :overflow)))

(let [converted (s/conform ::text-edit edit)]
  (assert (s/valid? ::text-edit edit))
  (assert (= edit converted))
  (assert (int-coordinates? converted))
  (assert (identical? (:new-text edit) (:new-text converted)))
  (assert (identical? converted (s/conform ::text-edit converted))))

(doseq [invalid [nil ::s/invalid []
                 (dissoc edit :new-text)
                 (assoc edit :new-text ::s/invalid)
                 (update edit :range dissoc :end)
                 (assoc-in edit [:range :start :line] 1.0)
                 (dissoc overflow-edit :new-text)]]
  (assert (not (s/valid? ::text-edit invalid)))
  (assert (= ::s/invalid (s/conform ::text-edit invalid))))

(s/def ::annotation string?)
(assert (= ::s/invalid (s/conform ::text-edit (assoc edit ::annotation false))))
(assert (= ::s/invalid (:annotation (s/conform ::text-edit (assoc edit :annotation ::s/invalid)))))

(doseq [spec [::additional-text-edits ::edits ::text-edits]]
  (doseq [empty-coll [[] () #{} {}]]
    (let [result (s/conform spec empty-coll)]
      (assert (= empty-coll result))
      (assert (= (type empty-coll) (type result)))))
  (let [result (s/conform spec clojure.lang.PersistentQueue/EMPTY)]
    (assert (list? result))
    (assert (empty? result)))
  (doseq [[input result-kind] [[[edit second-edit] vector?]
                               [(list edit second-edit) list?]
                               [(seq [edit second-edit]) list?]
                               [(lazy-seq (list edit second-edit)) list?]
                               [(conj clojure.lang.PersistentQueue/EMPTY edit second-edit) list?]]]
    (let [result (s/conform spec input)]
      (assert (result-kind result))
      (assert (= [edit second-edit] (vec result)))
      (assert (every? int-coordinates? result))))
  (let [converted (s/conform ::text-edit edit)
        unchanged (clojure.lang.MapEntry/create converted converted)
        changed (clojure.lang.MapEntry/create edit edit)
        result (s/conform spec changed)
        list-input (list converted)
        list-result (s/conform spec list-input)]
    (assert (= list-input list-result))
    (assert (not (identical? list-input list-result)))
    (assert (identical? converted (first list-result)))
    (assert (identical? unchanged (s/conform spec unchanged)))
    (assert (vector? result))
    (assert (not (map-entry? result)))
    (assert (= [edit edit] result))
    (assert (every? int-coordinates? result)))
  (let [result (s/conform spec #{edit second-edit})]
    (assert (set? result))
    (assert (= #{edit second-edit} result))
    (assert (every? int-coordinates? result)))
  (assert (= ::s/invalid (conform-or-overflow spec #{nil})))
  (assert (= :overflow (conform-or-overflow spec #{overflow-edit})))
  ;; A nil key occupies its own slot in the JVM hash set traversal.
  (assert (= ::s/invalid (conform-or-overflow spec #{nil overflow-edit})))
  ;; Both outcomes become observable when the invalid member has a regular hash.
  (let [invalid-edit (assoc edit :new-text nil)
        observed (into #{}
                       (for [tag (range 64)]
                         (conform-or-overflow spec #{(assoc invalid-edit :tag tag)
                                                     overflow-edit})))]
    (assert (= #{::s/invalid :overflow} observed)))
  (doseq [invalid [nil [::s/invalid] {:entry edit} (first {:entry edit})
                   (list nil overflow-edit) (seq [::s/invalid overflow-edit])]]
    (assert (= ::s/invalid (s/conform spec invalid))))
  (doseq [input [[overflow-edit nil]
                 (list overflow-edit nil)
                 (seq [overflow-edit nil])]]
    (assert (= :overflow (conform-or-overflow spec input)))))

(let [by-bigint-line (fn [left right]
                       (let [lines (map #(get-in % [:range :start :line]) [left right])]
                         (when-not (every? #(instance? clojure.lang.BigInt %) lines)
                           (throw (ex-info "Expected unconverted coordinates" {:reason :converted})))
                         (apply compare lines)))
      input (sorted-set-by by-bigint-line edit second-edit)]
  (assert (= :converted
             (try
               (s/conform ::text-edits input)
               (catch clojure.lang.ExceptionInfo error
                 (:reason (ex-data error)))))))

(prn :text-edit-conformance-ok)

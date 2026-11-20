(ns demo.workspace-changes (:require [clojure.spec.alpha :as s]))

;; The workspace change dictionary and its dependencies follow lsp4clj 2.0.1.
(s/def ::line (s/and integer? (s/conformer int)))
(s/def ::character (s/and integer? (s/conformer int)))
(s/def ::position (s/keys :req-un [::line ::character]))
(s/def ::start ::position)
(s/def ::end ::position)
(s/def ::range (s/keys :req-un [::start ::end]))
(s/def ::new-text string?)
(s/def ::text-edit (s/keys :req-un [::new-text ::range]))
(s/def ::edits (s/coll-of ::text-edit))
(s/def ::changes (s/coll-of (s/tuple string? ::edits) :kind map?))

;; JVM-only assertions. Run with clojure -M tests/fixtures/spec-workspace-changes.clj

(s/def ::change-entry (s/tuple string? ::edits))

(defn edit [offset]
  {:new-text "text"
   :range {:start {:line offset :character offset}
           :end {:line offset :character offset}}})

(defn outcome [operation input]
  (try (operation ::changes input)
       (catch ArithmeticException _ :arithmetic)
       (catch IllegalArgumentException _ :argument)))

(let [uri (String. "file:///old")
      canonical [(edit (int 2))]
      changing [(edit 2N)]
      overflow [(edit 2147483648)]
      outside-long [(edit 9223372036854775808N)]
      same-entry (clojure.lang.MapEntry/create uri canonical)
      changed-entry (clojure.lang.MapEntry/create uri changing)
      conformed-entry (s/conform ::change-entry changed-entry)]
  (assert (identical? same-entry (s/conform ::change-entry same-entry)))
  (assert (vector? conformed-entry))
  (assert (not (map-entry? conformed-entry)))
  (assert (identical? uri (first conformed-entry)))
  (assert (= java.lang.Integer (class (get-in conformed-entry [1 0 :range :start :line]))))
  (assert (= ::s/invalid (s/conform ::change-entry [uri])))
  (assert (= ::s/invalid (s/conform ::change-entry (list uri canonical))))

  (doseq [[construct map-class] [[array-map clojure.lang.PersistentArrayMap]
                                 [hash-map clojure.lang.PersistentHashMap]
                                 [sorted-map clojure.lang.PersistentTreeMap]]
          [edits same-map?] [[[] true]
                             [canonical true]
                             [changing false]
                             [[(edit 2)] false]
                             [[(edit (int 256))] false]
                             [(list (first canonical)) false]]]
    (let [input (with-meta (construct uri edits) {::marked true})
          output (s/conform ::changes input)]
      (assert (= map-class (class output)))
      (assert (= {::marked true} (meta output)))
      (assert (identical? uri (ffirst output)))
      (assert (= same-map? (identical? input output))
              (str construct " " (pr-str edits)))
      (assert (s/valid? ::changes input))))

  (doseq [input [{} (hash-map) (sorted-map)]]
    (assert (identical? input (s/conform ::changes input))))
  (doseq [input [nil [] [same-entry] same-entry
                 {:bad-key overflow} {uri :bad-edits}
                 {uri [{:new-text "missing range"}]}]]
    (assert (= ::s/invalid (s/conform ::changes input)))
    (assert (false? (s/valid? ::changes input))))

  (doseq [operation [s/conform s/valid?]]
    (let [invalid (if (= operation s/conform) ::s/invalid false)]
      (assert (= :arithmetic (outcome operation {uri overflow})))
      (assert (= :argument (outcome operation {uri outside-long})))
      (assert (= invalid (outcome operation (array-map :bad-key overflow uri overflow))))
      (assert (= :arithmetic (outcome operation (array-map uri overflow :bad-key overflow)))))))

(let [a (String. "file:///a")
      b (String. "file:///b")
      changing [(edit 2N)]
      wide [(edit 2147483648)]
      outside-long [(edit 9223372036854775808N)]]
  (doseq [make-map [array-map hash-map sorted-map]]
    (let [input (make-map b [] a (with-meta changing {::marked true}))
          result (s/conform ::changes input)]
      (assert (= (vec (keys input)) (vec (keys result))))
      (assert (every? true? (map identical? (keys input) (keys result))))
      (assert (= {::marked true} (meta (get result a))))))
  (doseq [operation [s/conform s/valid?]]
    (let [invalid (if (= operation s/conform) ::s/invalid false)]
      (doseq [bad [nil false ::s/invalid]
              input [bad {bad wide} {a bad} {a [bad]}]]
        (assert (= invalid (outcome operation input))))
      (assert (= invalid (outcome operation {:invalid outside-long})))
      (assert (= :arithmetic (outcome operation (array-map a wide b outside-long))))
      (assert (= :argument (outcome operation (array-map b outside-long a wide)))))))

(defrecord ChangeMap [])
(let [input (with-meta (assoc (->ChangeMap) "file:///record" [(edit 2N)]) {::marked true})
      result (s/conform ::changes input)]
  (assert (instance? ChangeMap result))
  (assert (= (meta input) (meta result)))
  (assert (= input result))
  (assert (instance? Integer (get-in result ["file:///record" 0 :range :start :line])))
  (assert (s/valid? ::changes input))
  (assert (= ::s/invalid (s/conform ::changes (assoc input :keyword [(edit 2147483648)])))))

;; Tuple key conformance still runs. By default the result map retains the original key.
(let [visited (atom [])
      key-spec (s/conformer (fn [value]
                              (swap! visited conj [:key value])
                              (if (string? value) :converted ::s/invalid)))
      value-spec (s/conformer (fn [value] (swap! visited conj [:value value]) (inc value)))
      by-default (s/coll-of (s/tuple key-spec value-spec) :kind map?)
      conforming-keys (s/coll-of (s/tuple key-spec value-spec) :kind map? :conform-keys true)]
  (assert (= ::s/invalid (s/conform by-default {:invalid 7})))
  (assert (= [[:key :invalid]] @visited))
  (reset! visited [])
  (assert (= {"file:///example" 8} (s/conform by-default {"file:///example" 7})))
  (assert (= [[:key "file:///example"] [:value 7]] @visited))
  (assert (= {:converted 8} (s/conform conforming-keys {"file:///example" 7}))))

;; An empty map does not force tuple slots. The first nonempty entry resolves both specs.
(let [late (s/coll-of (s/tuple string? ::late-edits) :kind map?)
      key-rejects {:invalid [(edit 2147483648)]}
      input {"file:///example" [(edit 2N)]}
      resolve-error (fn [f]
                      (try (f) (catch Exception error error)))]
  (assert (= {} (s/conform late {})))
  (let [failure (resolve-error #(s/conform late key-rejects))]
    (assert (instance? Exception failure))
    (assert (= (str "Unable to resolve spec: " ::late-edits) (ex-message failure)))
    (s/def ::late-edits ::edits)
    (assert (identical? failure (resolve-error #(s/valid? late input))))
    (let [fresh (s/coll-of (s/tuple string? ::late-edits) :kind map?)]
      (assert (= ::s/invalid (s/conform fresh key-rejects)))
      (assert (s/valid? fresh input))
      (s/def ::late-edits string?)
      (assert (s/valid? fresh input))
      (assert (= ::s/invalid (s/conform fresh {"file:///example" "new registration"})))
      (assert (s/valid? (s/coll-of (s/tuple string? ::late-edits) :kind map?)
                        {"file:///example" "new registration"})))
    (s/def ::late-edits nil)))

(prn :workspace-changes-ok (clojure-version))

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

(prn :workspace-changes-ok (clojure-version))

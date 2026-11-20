(ns demo.document-changes (:require [clojure.spec.alpha :as s]))

;; Resource operations and the text edit dependency shapes follow lsp4clj 2.0.1.
(s/def ::line (s/and integer? (s/conformer int)))
(s/def ::character (s/and integer? (s/conformer int)))
(s/def ::position (s/keys :req-un [::line ::character]))
(s/def ::start ::position)
(s/def ::end ::position)
(s/def ::range (s/keys :req-un [::start ::end]))
(s/def ::new-text string?)
(s/def ::text-edit (s/keys :req-un [::new-text ::range]))
(s/def ::edits (s/coll-of ::text-edit))
(s/def ::version (s/and integer? (s/conformer int)))
(s/def ::uri string?)
(s/def ::text-document (s/keys :req-un [::version ::uri]))
(s/def ::text-document-edit (s/keys :req-un [::text-document ::edits]))

(s/def :create-file/options (s/keys :opt-un [::overwrite ::ignore-if-exists]))
(s/def :create-file/kind (s/and string? #(= % "create")))
(s/def ::create-file (s/keys :req-un [:create-file/kind ::uri] :opt-un [:create-file/options]))
(s/def :rename-file/kind (s/and string? #(= % "rename")))
(s/def :rename-file/old-uri ::uri)
(s/def :rename-file/new-uri ::uri)
(s/def ::rename-file (s/keys :req-un [:rename-file/kind :rename-file/old-uri :rename-file/new-uri]))

(s/def ::document-changes-entry
  (s/or :create-file ::create-file :rename-file ::rename-file :text-document-edit ::text-document-edit))
(s/def ::document-changes
  (s/and (s/coll-of ::document-changes-entry) (s/conformer #(map second %))))
(s/def ::changes (s/coll-of (s/tuple string? ::edits) :kind map?))
(s/def ::workspace-edit (s/keys :opt-un [::document-changes ::changes]))

;; JVM-only assertions. Run with clojure -M tests/fixtures/spec-document-changes.clj

(s/def ::tagged-changes (s/coll-of ::document-changes-entry))
(s/def ::workspaces (s/coll-of ::workspace-edit))

(def create-input {:kind (String. "create") :uri "file:///new" :options {:overwrite false}})
(def rename-input {:kind (String. "rename") :old-uri "file:///old" :new-uri "file:///new"})
(def text-input {:text-document {:uri "file:///old" :version 1N}
                 :edits [{:new-text "x"
                          :range {:start {:line 2N :character 0} :end {:line 2N :character 1}}}]})
(def raw [create-input rename-input text-input])
(def wide (assoc-in text-input [:text-document :version] 2147483648))

(defn outcome [operation input]
  (try (operation ::document-changes input)
       (catch ArithmeticException _ :overflow)))

(doseq [[input expected] [[raw raw]
                          [(apply list raw) raw]
                          [(seq raw) raw]
                          [(clojure.lang.MapEntry/create create-input rename-input)
                           [create-input rename-input]]
                          [[] []]
                          ['() []]
                          [{} []]
                          [#{} []]]]
  (let [result (s/conform ::document-changes input)]
    (assert (= clojure.lang.LazySeq (class result)))
    (assert (s/valid? ::document-changes input))
    (assert (= expected (vec result)))
    (assert (not (identical? input result)))
    (assert (= (empty? expected) (nil? (seq result))))
    (when (seq expected)
      (assert (identical? (first expected) (first result))))))

(let [normalized (last (s/conform ::document-changes raw))]
  (assert (not (identical? text-input normalized)))
  (assert (= Integer (class (get-in normalized [:text-document :version]))))
  (assert (= Integer (class (get-in normalized [:edits 0 :range :start :line])))))

(doseq [input [nil false "create" create-input {:entry create-input} {create-input rename-input}
               [nil] [::s/invalid] [[:create-file create-input]] #{nil}]]
  (assert (not (s/valid? ::document-changes input)))
  (assert (= ::s/invalid (s/conform ::document-changes input))))

(doseq [op [s/conform s/valid?]
        [input expected] [[[wide nil] :overflow]
                          [[nil wide] (if (identical? op s/valid?) false ::s/invalid)]
                          [(list wide nil) :overflow]
                          [(seq [wide nil]) :overflow]
                          [#{wide} :overflow]
                          [(clojure.lang.MapEntry/create wide nil) :overflow]
                          [(clojure.lang.MapEntry/create nil wide)
                           (if (identical? op s/valid?) false ::s/invalid)]]]
  (assert (= expected (outcome op input))))

(let [input #{text-input}
      result (s/conform ::document-changes input)
      [entry] result]
  (assert (= clojure.lang.LazySeq (class result)))
  (assert (s/valid? ::document-changes input))
  (assert (= [text-input] (vec result)))
  (assert (not (identical? text-input entry)))
  (assert (= Integer (class (get-in entry [:text-document :version]))))
  (assert (= Integer (class (get-in entry [:edits 0 :range :start :line])))))

;; Hash-set traversal can reach either failure first, even for the same literal order.
(let [invalid :invalid-document-change]
  (doseq [[uri overflow-first?] [["file:///overflow-0" true]
                                ["file:///overflow-9" false]]
          :let [overflow (assoc-in wide [:text-document :uri] uri)
                input #{invalid overflow}]]
    (assert (= (if overflow-first? overflow invalid) (first input)))
    (assert (= (if overflow-first? :overflow ::s/invalid) (outcome s/conform input)))
    (assert (= (if overflow-first? :overflow false) (outcome s/valid? input)))))

(let [input (set raw)
      result (s/conform ::document-changes input)
      tagged (s/conform ::tagged-changes input)]
  (assert (= clojure.lang.LazySeq (class result)))
  (assert (s/valid? ::document-changes input))
  (assert (= input (set result)))
  (assert (= (vec (map second tagged)) (vec result))))

(let [input (set (for [n (range 16)] (assoc rename-input :new-uri (str "file:///" n))))
      result (s/conform ::document-changes input)
      before (mapv :new-uri input)
      after (mapv :new-uri result)]
  (assert (= (set before) (set after)))
  (assert (not= before after)))

(def collapse-values {1 :same 2 :same})
(s/def :extra/collapse (s/and int? collapse-values (s/conformer collapse-values)))
(let [input #{(assoc create-input :extra/collapse 1) (assoc create-input :extra/collapse 2)}
      result (s/conform ::document-changes input)]
  (assert (= 2 (count input)))
  (assert (= 1 (count result)))
  (assert (= :same (:extra/collapse (first result)))))

(let [empty-edit {}
      changes-only {:changes {"file:///old" []}}
      doc-edit {:document-changes raw}
      output (s/conform ::workspace-edit doc-edit)
      empty-doc-edit {:document-changes []}
      empty-output (s/conform ::workspace-edit empty-doc-edit)
      entry (clojure.lang.MapEntry/create empty-edit empty-doc-edit)
      rebuilt (s/conform ::workspaces entry)]
  (assert (identical? empty-edit (s/conform ::workspace-edit empty-edit)))
  (assert (s/valid? ::workspace-edit changes-only))
  (assert (s/valid? ::workspace-edit doc-edit))
  (assert (s/valid? ::workspace-edit empty-doc-edit))
  (assert (not (identical? doc-edit output)))
  (assert (= clojure.lang.LazySeq (class (:document-changes output))))
  (assert (= raw (vec (:document-changes output))))
  (assert (identical? create-input (first (:document-changes output))))
  (assert (= clojure.lang.LazySeq (class (:document-changes empty-output))))
  (assert (nil? (seq (:document-changes empty-output))))
  (assert (vector? rebuilt))
  (assert (not (map-entry? rebuilt)))
  (assert (identical? empty-edit (first rebuilt)))
  (assert (= empty-output (second rebuilt)))
  (assert (= ::s/invalid (s/conform ::workspace-edit {:document-changes {:entry create-input}})))
  (assert (= :overflow (try (s/conform ::workspace-edit {:document-changes [wide]})
                           (catch ArithmeticException _ :overflow)))))
(prn :document-changes-projection-ok (clojure-version))

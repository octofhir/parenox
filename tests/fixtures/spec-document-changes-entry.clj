(ns demo.document-changes-entry (:require [clojure.spec.alpha :as s]))

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
(s/def ::text-document-first
  (s/or :text-document-edit ::text-document-edit :create-file ::create-file))

;; JVM-only assertions. Run with clojure -M tests/fixtures/spec-document-changes-entry.clj

(s/def ::wrapper (s/keys :req-un [::document-changes-entry]))
(s/def ::wrapped (s/coll-of ::wrapper))
(s/def ::marker-tag (s/or ::s/invalid ::create-file))

(def create-input {:kind (String. "create") :uri (String. "file:///new") :options {:overwrite false}})
(def rename-input {:kind (String. "rename") :old-uri "file:///old" :new-uri "file:///new"})
(def text-input {:text-document {:uri "file:///old" :version 1N}
                 :edits [{:new-text "x" :range {:start {:line 2N :character 0}
                                               :end {:line 2N :character 1}}}]})
(defn outcome [f]
  (try (f) (catch ArithmeticException _ :overflow)))

(doseq [[expected input] [[:create-file create-input]
                          [:rename-file rename-input]
                          [:text-document-edit text-input]]]
  (let [output (s/conform ::document-changes-entry input)]
    (assert (s/valid? ::document-changes-entry input))
    (assert (vector? output))
    (assert (map-entry? output))
    (assert (= clojure.lang.MapEntry (class output)))
    (assert (= expected (key output)))
    (assert (identical? (val output) (second output)))
    (assert (not (identical? output (s/conform ::document-changes-entry input))))
    (assert (= expected (first output)))
    (assert (= input (second output)))
    (assert (= (not= :text-document-edit expected) (identical? input (second output))))))
(let [[_ output] (s/conform ::document-changes-entry text-input)]
  (assert (= Integer (class (get-in output [:text-document :version]))))
  (assert (= Integer (class (get-in output [:edits 0 :range :start :line])))))

(doseq [input [nil false ::s/invalid [] [:create-file create-input] {} {:kind "rename" :uri "file:///new"}]]
  (assert (not (s/valid? ::document-changes-entry input)))
  (assert (= ::s/invalid (s/conform ::document-changes-entry input))))

(let [wide (assoc-in text-input [:text-document :version] 2147483648)
      overlap (merge create-input wide)
      falling-through (assoc overlap :kind "other")]
  (assert (= :create-file (first (s/conform ::document-changes-entry overlap))))
  (assert (s/valid? ::document-changes-entry overlap))
  (assert (= :overflow (outcome #(s/conform ::text-document-first overlap))))
  (assert (= :overflow (outcome #(s/conform ::document-changes-entry falling-through))))
  (assert (= :overflow (outcome #(s/valid? ::document-changes-entry falling-through)))))

(let [left {:document-changes-entry create-input}
      right {:document-changes-entry rename-input}
      entry (clojure.lang.MapEntry/create left right)
      rebuilt (s/conform ::wrapped entry)]
  (assert (vector? rebuilt))
  (assert (not (map-entry? rebuilt)))
  (assert (identical? create-input (get-in rebuilt [0 :document-changes-entry 1])))
  (assert (identical? rename-input (get-in rebuilt [1 :document-changes-entry 1]))))
(assert (= [::s/invalid create-input] (s/conform ::marker-tag create-input)))

(s/def ::eager (s/or :create ::create-file :future ::late-map))
(s/def ::lazy (s/or :create ::create-file :future ::late-map))
(defn unresolved? [f]
  (try (f) false
       (catch Exception e
         (.contains (.getMessage e) "Unable to resolve spec: :demo.document-changes-entry/late-map"))))
(s/def ::probe (s/with-gen string?
                 (do (assert (unresolved? #(s/conform ::eager create-input)))
                     (fn [] nil))))
(s/def ::late-map (s/keys :req-un [::version]))
(assert (unresolved? #(s/conform ::eager create-input)))
(assert (= [:create create-input] (s/conform ::lazy create-input)))
(prn :document-changes-entry-jvm-ok (clojure-version))

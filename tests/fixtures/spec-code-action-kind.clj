(ns demo.code-action-kind (:require [clojure.spec.alpha :as s]))

;; The table and tagged kind spec follow lsp4clj 2.0.1.
(def code-action-kind
  {:quick-fix "quickfix"
   :refactor "refactor"
   :refactor-extract "refactor.extract"
   :refactor-inline "refactor.inline"
   :refactor-rewrite "refactor.rewrite"
   :source "source"
   :source-organize-imports "source.organizeImports"})

(s/def :code-action/kind
  (s/and (s/or :keyword (s/and keyword? code-action-kind (s/conformer code-action-kind))
               :string string?)
         (s/conformer second)))
(s/def ::kind :code-action/kind)

(def edge-kinds {:empty "" :false false :nil nil :marker ::s/invalid ::s/invalid "reserved key"})
(s/def ::edge-kind
  (s/and (s/or :keyword (s/and keyword? edge-kinds (s/conformer edge-kinds))
               :string string?)
         (s/conformer second)))

;; Only the action title and kind are needed to exercise the field conversion.
(s/def :code-action/title string?)
(s/def ::code-action (s/keys :req-un [:code-action/title] :opt-un [:code-action/kind]))
(s/def ::code-actions (s/coll-of ::code-action))
(s/def ::action-batch (s/keys :req-un [::code-action]))

;; JVM-only assertions. Run with clojure -M tests/fixtures/spec-code-action-kind.clj

(doseq [spec [:code-action/kind ::kind]
        [input output] code-action-kind]
  (assert (s/valid? spec input))
  (assert (identical? output (s/conform spec input))))

(doseq [text ["workspace.myAction" "" "source.organizeImports" ":quick-fix" ":clojure.spec.alpha/invalid"]]
  (let [input (String. text)]
    (assert (not (identical? text input)))
    (assert (s/valid? :code-action/kind input))
    (assert (identical? input (s/conform :code-action/kind input)))))

(doseq [input [:unlisted :source-organizeimports ::s/invalid nil false true 1 'quick-fix [] {}]]
  (assert (not (s/valid? :code-action/kind input)))
  (assert (= ::s/invalid (s/conform :code-action/kind input))))

(assert (s/valid? ::edge-kind :empty))
(assert (identical? (:empty edge-kinds) (s/conform ::edge-kind :empty)))
(doseq [input [:false :nil :marker ::s/invalid]]
  (assert (not (s/valid? ::edge-kind input)))
  (assert (= ::s/invalid (s/conform ::edge-kind input))))

(let [custom (String. "workspace.myAction")
      as-text {:title "Existing kind" :kind custom}
      as-keyword {:title "Organize imports" :kind :source-organize-imports}
      unchanged {:title "No kind"}
      expected {:title "Organize imports" :kind "source.organizeImports"}
      retained (clojure.lang.MapEntry/create as-text unchanged)
      rebuilt (s/conform ::code-actions (clojure.lang.MapEntry/create as-text as-keyword))
      text-batch {:code-action as-text}]
  (assert (s/valid? ::code-action as-keyword))
  (assert (s/valid? ::action-batch {:code-action as-keyword}))
  (assert (identical? as-text (s/conform ::code-action as-text)))
  (assert (identical? unchanged (s/conform ::code-action unchanged)))
  (assert (identical? text-batch (s/conform ::action-batch text-batch)))
  (assert (= {:code-action expected} (s/conform ::action-batch {:code-action as-keyword})))
  (assert (identical? retained (s/conform ::code-actions retained)))
  (assert (vector? rebuilt))
  (assert (not (map-entry? rebuilt)))
  (assert (identical? as-text (first rebuilt)))
  (assert (= expected (second rebuilt)))
  (assert (identical? (:source-organize-imports code-action-kind) (:kind (second rebuilt))))
  (doseq [input [{:title "unknown" :kind :unlisted} {:title "nil" :kind nil} {:kind :quick-fix}]]
    (assert (not (s/valid? ::code-action input)))
    (assert (= ::s/invalid (s/conform ::code-action input)))))

(prn :code-action-kind-spec-ok (clojure-version))

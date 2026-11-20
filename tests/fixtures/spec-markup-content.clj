(ns demo.markup-content (:require [clojure.spec.alpha :as s]))

;; The two predicates and required fields follow lsp4clj 2.0.1.
(s/def :markup/kind #{"plaintext" "markdown"})
(s/def :markup/value string?)
(s/def ::markup-content (s/keys :req-un [:markup/kind :markup/value]))
(s/def ::contents (s/coll-of ::markup-content))
(s/def ::kind :markup/kind)

;; JVM-only assertions. Run with clojure -M tests/fixtures/spec-markup-content.clj

(doseq [kind ["markdown" "plaintext"]]
  (let [input (String. kind)
        table #{kind}]
    (assert (not (identical? kind input)))
    (assert (identical? kind (table input)))
    (assert (s/valid? :markup/kind input))
    (assert (identical? input (s/conform :markup/kind input)))
    (assert (identical? input (s/conform ::kind input)))))

(doseq [input ["" "Markdown" "markdown " :markdown 'markdown nil false true ::s/invalid 0 [] {}]]
  (assert (not (s/valid? :markup/kind input)))
  (assert (= ::s/invalid (s/conform :markup/kind input))))

(let [left {:kind (String. "markdown") :value (String. "contents") :extra/unregistered ::s/invalid}
      right {:kind (String. "plaintext") :value (String. "plain") :other nil
             :markup/kind (String. "markdown")}
      entry (clojure.lang.MapEntry/create left right)]
  (assert (s/valid? ::markup-content left))
  (assert (identical? left (s/conform ::markup-content left)))
  (assert (identical? right (s/conform ::markup-content right)))
  (assert (identical? entry (s/conform ::contents entry)))
  (assert (= ::s/invalid (s/conform ::markup-content (assoc right :markup/kind "other"))))
  (assert (= ::s/invalid (s/conform ::contents (clojure.lang.MapEntry/create left {})))))

(doseq [input [{:kind "markdown"} {:value "x"} {:kind "other" :value "x"}
               {:kind "markdown" :value nil} {:kind "markdown" :value ::s/invalid}
               {:markup/kind "markdown" :value "x"}]]
  (assert (= ::s/invalid (s/conform ::markup-content input))))

(s/def ::mixed #{"markdown" nil false ::s/invalid})
(doseq [input [nil false ::s/invalid]]
  (assert (not (s/valid? ::mixed input)))
  (assert (= ::s/invalid (s/conform ::mixed input))))

(prn :markup-content-spec-ok (clojure-version))

(ns demo.documentation
  (:require [clojure.spec.alpha :as s]))

;; The named projection and markup spec follow lsp4clj 2.0.1.
(s/def ::documentation
  (s/and (s/or :string string? :markup-content ::markup-content) (s/conformer second)))
(s/def :markup/kind #{"plaintext" "markdown"})
(s/def :markup/value string?)
(s/def ::markup-content (s/keys :req-un [:markup/kind :markup/value]))
(s/def ::map-first
  (s/and (s/or :markup-content ::markup-content :string clojure.core/string?)
         (s/conformer clojure.core/second)))
(s/def ::wrapped (s/keys :req-un [::documentation]))
(s/def ::wrapped-docs (s/coll-of ::wrapped))
(s/def :extra/line (s/and integer? (s/conformer int)))

;; JVM-only assertions. Run with clojure -M tests/fixtures/spec-documentation.clj

(let [text (String. "docs")
      markup {:kind (String. "markdown") :value (String. "content")}
      lhs {:documentation text}
      rhs {:documentation markup}
      entry (clojure.lang.MapEntry/create lhs rhs)]
  (doseq [spec [::documentation ::map-first]]
    (assert (s/valid? spec text))
    (assert (identical? text (s/conform spec text)))
    (assert (s/valid? spec markup))
    (assert (identical? markup (s/conform spec markup))))
  (assert (identical? entry (s/conform ::wrapped-docs entry)))
  (let [raw (assoc markup :extra/line 1N)
        converted (s/conform ::documentation raw)
        changed (s/conform ::wrapped-docs
                           (clojure.lang.MapEntry/create lhs {:documentation raw}))]
    (assert (= raw converted))
    (assert (not (identical? raw converted)))
    (assert (= Integer (type (:extra/line converted))))
    (assert (vector? changed))
    (assert (not (map-entry? changed)))
    (assert (identical? lhs (first changed)))
    (assert (= {:documentation converted} (second changed))))
  (assert (= ::s/invalid (s/conform ::wrapped-docs (clojure.lang.MapEntry/create lhs {})))))

(doseq [spec [::documentation ::map-first]
        input [nil false ::s/invalid :markdown 1 [] {} {:kind "markdown"}
               {:kind "other" :value "x"} {:kind "plaintext" :value false}
               {:kind "markdown" :value "x" :extra/line nil}]]
  (assert (not (s/valid? spec input)))
  (assert (= ::s/invalid (s/conform spec input))))

(defn outcome [attempt]
  (try (attempt)
       (catch ArithmeticException _ :overflow)))

(assert (= ::s/invalid (outcome #(s/conform ::documentation {:extra/line 2147483648}))))
(assert (= :overflow (outcome #(s/conform ::documentation
                                        (array-map :extra/line 2147483648 :kind "unknown" :value "x")))))
(assert (= ::s/invalid (outcome #(s/conform ::documentation
                                          (array-map :kind "unknown" :extra/line 2147483648 :value "x")))))
(assert (= :overflow (outcome #(s/valid? ::documentation
                                       {:kind "markdown" :value "x" :extra/line 2147483648}))))

(defn missing-spec? [attempt]
  (try (attempt) false
       (catch Exception error
         (.contains (.getMessage error) "Unable to resolve spec: :demo.documentation/late"))))

(s/def ::early (s/and (s/or :string string? :markup-content ::late) (s/conformer second)))
(assert (missing-spec? #(s/def ::premature-alias ::late)))
(s/def ::early-probe
  (s/with-gen string?
    (do
      (assert (missing-spec? #(s/conform ::early "still resolves the map branch")))
      (fn [] nil))))
(s/def ::fresh (s/and (s/or :string string? :markup-content ::ready-alias) (s/conformer second)))
(s/def ::late (s/keys :req-un [:markup/kind :markup/value]))
(assert (missing-spec? #(s/conform ::early "failed resolution is retained")))
(s/def ::ready-alias ::late)
(assert (= "works after registration" (s/conform ::fresh "works after registration")))

(prn :documentation-spec-ok (clojure-version))

(ns demo.file-kind (:require [clojure.spec.alpha :as s]))

;; The resource operations use the kind predicates and map shapes from lsp4clj 2.0.1.
(s/def ::uri string?)
(s/def :create-file/options (s/keys :opt-un [::overwrite ::ignore-if-exists]))
(s/def :create-file/kind (s/and string? #(= % "create")))
(s/def ::create-file (s/keys :req-un [:create-file/kind ::uri] :opt-un [:create-file/options]))
(s/def :rename-file/kind (s/and string? #(= % "rename")))
(s/def :rename-file/old-uri ::uri)
(s/def :rename-file/new-uri ::uri)
(s/def ::rename-file (s/keys :req-un [:rename-file/kind :rename-file/old-uri :rename-file/new-uri]))

(s/def ::rename-kind :rename-file/kind)
(s/def ::reversed (s/and string? #(clojure.core/= "create" %1)))
(s/def ::emoji (s/and string? #(= % "\ud83e\uddea")))
(s/def ::accent (s/and string? #(= "é" %1)))
(s/def ::batch (s/keys :req-un [::create-file ::rename-file]))
(s/def ::renames (s/coll-of ::rename-file))

;; JVM-only assertions. Run with clojure -M tests/fixtures/spec-file-kind.clj

(doseq [[spec kind other] [[:create-file/kind "create" "rename"]
                          [:rename-file/kind "rename" "create"]
                          [::rename-kind "rename" "create"]
                          [::reversed "create" "rename"]]]
  (let [input (String. kind)]
    (assert (not (identical? kind input)))
    (assert (s/valid? spec input))
    (assert (identical? input (s/conform spec input))))
  (doseq [input [other "" (str kind " ") (str " " kind) (keyword kind) (symbol kind)
                 \c nil false true 1 [] {} ::s/invalid]]
    (assert (not (s/valid? spec input)))
    (assert (= ::s/invalid (s/conform spec input)))))
(assert (not (s/valid? :create-file/kind "Create")))
(assert (not (s/valid? :rename-file/kind "Rename")))

(let [create {:kind (String. "create") :uri (String. "file:///created")
              :options {:overwrite false :ignore-if-exists nil}}
      rename {:kind (String. "rename") :old-uri (String. "file:///old") :new-uri (String. "file:///new")}
      batch {:create-file create :rename-file rename}
      entry (clojure.lang.MapEntry/create rename rename)]
  (doseq [[spec input] [[::create-file create] [::rename-file rename] [::batch batch] [::renames entry]]]
    (assert (s/valid? spec input))
    (assert (identical? input (s/conform spec input))))
  (assert (= ::s/invalid (s/conform ::create-file (dissoc create :kind))))
  (assert (= ::s/invalid (s/conform ::create-file (assoc create :kind "rename"))))
  (assert (= ::s/invalid (s/conform ::create-file (assoc create :options false))))
  (assert (= ::s/invalid (s/conform ::rename-file (dissoc rename :new-uri))))
  (assert (= ::s/invalid (s/conform ::rename-file (assoc rename :kind "create"))))
  (assert (= ::s/invalid (s/conform ::batch (assoc-in batch [:rename-file :kind] :rename)))))

(assert (s/valid? ::emoji "🧪"))
(assert (s/valid? ::emoji "\ud83e\uddea"))
(assert (s/valid? ::accent "\u00e9"))
(assert (not (s/valid? ::accent "e\u0301")))

(s/def ::lone-surrogate (s/and string? #(= % "\ud800")))
(assert (s/valid? ::lone-surrogate "\ud800"))
(assert (not (s/valid? ::lone-surrogate "\ud801")))

(ns demo.file-kind-shadow
  (:refer-clojure :exclude [=])
  (:require [clojure.spec.alpha :as s]))
(defn = [_ _] true)
(s/def ::local (s/and string? #(= % "create")))
(s/def ::canonical (s/and string? #(clojure.core/= % "create")))
(assert (s/valid? ::local "rename"))
(assert (not (s/valid? ::canonical "rename")))

(prn :file-kind-spec-ok (clojure-version))

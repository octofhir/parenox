(ns requirements
  (:require [clojure.spec.alpha :as s]
            [medley.core :as medley]))

(defn lib-requirements
  {:t/sig '[symbol? (s/coll-of string? :kind vector?) :-> {:lib string? :total number?}]}
  [lib usages]
  {:lib (str lib) :total (count usages)})

;; The callback shape follows clojure-lsp's form-requirements.
(defn lib->requirements
  {:t/sig '[(s/map-of symbol? (s/coll-of string? :kind vector?))
            :-> (s/map-of symbol? {:lib string? :total number?})]}
  [lib->usages]
  (->> lib->usages
       (medley/map-kv-vals (fn [lib usages] (lib-requirements lib usages)))))

(defn labels
  {:t/sig '[(s/map-of string? number?) :-> (s/map-of string? string?)]}
  [labels]
  (medley/map-kv-vals #(str (subs %1 0) (inc %2)) labels))

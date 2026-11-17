(ns demo.portable
  (:require [clojure.spec.alpha :as s]
            #?@(:clj [[clojure.string :as str]]
                :cljs [[cljs.string :as str]])))

;; A browser-only definition does not enter the JVM interface or dependency graph.
#?(:cljs (defn browser-label [] (js/document.title)))

(defn label
  {:t/sig '[string? :-> string?]}
  [name]
  #?(:clj (str/upper-case name)
     :cljs (js/browser-label name)))

(defn separators
  {:t/sig '[:-> (s/coll-of char? :kind vector?)]}
  []
  [#?@(:clj [\space \-]
        :cljs [\tab])])

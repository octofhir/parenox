(ns demo.quoted
  (:require [clojure.spec.alpha :as s]))

(def labels '("primary" "fallback"))
(def settings (quote {:enabled false :fallback ("guest" "anon")}))

(defn fallback-label {:t/sig '[:-> string?]} []
  (nth labels 1))

(defn command-tag {:t/sig '[:-> symbol?]} []
  (first '(demo.external/command "ready")))

(defn modes {:t/sig '[:-> (s/coll-of keyword? :kind vector?)]} []
  '[:online :offline])

(defn guest-label {:t/sig '[:-> string?]} []
  (first (:fallback settings)))

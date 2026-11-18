(ns demo.generic-zip
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(defn zip-with
  {:t/sig '(t/all [a b c]
             [(s/fspec :args (s/cat :left a :right b) :ret c)
              (s/coll-of a :kind vector?) (s/coll-of b :kind vector?)
              :-> (s/coll-of c :kind vector?)])}
  [combine left right]
  (mapv combine left right))

(defn reading
  {:t/sig '(t/all [a b] [a b :-> {:label a :value b}])}
  [label value]
  {:label label :value value})

(defn readings
  {:t/sig '[(s/coll-of string? :kind vector?) (s/coll-of number? :kind vector?)
            :-> (s/coll-of {:label string? :value number?} :kind vector?)]}
  [labels values]
  (zip-with reading labels values))

(defn describe
  {:t/sig '[(s/coll-of string? :kind vector?) (s/coll-of number? :kind vector?)
            :-> (s/coll-of string? :kind vector?)]}
  [labels values]
  (zip-with (fn [label value] (str (str/trim label) ": " (inc value))) labels values))

(defn find-value
  {:t/sig '(t/all [a] [(s/map-of keyword? a) keyword? :-> (s/nilable a)])}
  [values key]
  (get values key))

(defn label-or-empty
  {:t/sig '[(s/map-of keyword? string?) :-> string?]}
  [labels]
  (if-some [text (find-value labels :daily)]
    (str/trim text)
    ""))

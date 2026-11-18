(ns demo.list-accumulators
  (:require [clojure.spec.alpha :as s]))

(s/def ::names (s/coll-of string? :kind vector?))
(s/def ::reversed (s/coll-of string? :kind seq?))

(defn reverse-names {:t/sig '[::names :-> ::reversed]} [names]
  (loop [remaining (seq names) output (list)]
    (if remaining
      (recur (next remaining) (cons (first remaining) output))
      output)))

(defn second-status {:t/sig '[:-> #{:paused}]} []
  (first (next (list :ready :paused))))

(defn head-status {:t/sig '[:-> #{:requested}]} []
  (first
    (try
      (cons :requested [:queued :sent])
      (catch ArithmeticException _ (list :unexpected)))))

(defn copy-names {:t/sig '[::names :-> ::reversed]} [names]
  (apply list names))

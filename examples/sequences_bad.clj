(ns demo.sequences-bad
  (:require [clojure.spec.alpha :as s]))

(defn increment-head
  {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [amounts]
  (inc (first amounts)))

(defn start-with-false {:t/sig '[:-> nil?]} []
  (seq false))

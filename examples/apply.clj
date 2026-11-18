(ns demo.apply
  (:require [clojure.spec.alpha :as s]
            [demo.arities.provider :as provider]
            [demo.variadic :as variadic]))

(defn fallback {:t/sig '[:-> string?]} []
  (apply provider/describe nil))

(defn offset {:t/sig '[:-> number?]} []
  (apply variadic/adjust 4 (next [0 3])))

(defn join-args {:t/sig '[:-> string?]} []
  (apply (fn [prefix & labels] (str prefix (first labels))) "queue/" ["ready"]))

(defn maybe-offset {:t/sig '[boolean? :-> number?]} [extra?]
  (apply variadic/adjust 4 (if extra? [3] nil)))

(defn increment-default {:t/sig '[:-> number?]} []
  (apply (fnil inc 0) [nil]))

(defn total-offset
  {:t/sig '[number? (s/coll-of number? :kind vector?) :-> number?]}
  [base deltas]
  (apply variadic/add-deltas base deltas))

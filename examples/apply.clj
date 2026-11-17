(ns demo.apply
  (:require [demo.arities.provider :as provider]
            [demo.variadic :as variadic]))

(defn fallback {:t/sig '[:-> string?]} []
  (apply provider/describe nil))

(defn offset {:t/sig '[:-> number?]} []
  (apply variadic/adjust 4 (next [0 3])))

(defn join-args {:t/sig '[:-> string?]} []
  (apply (fn [prefix & labels] (str prefix (first labels))) "queue/" ["ready"]))

(defn maybe-offset {:t/sig '[boolean? :-> number?]} [extra?]
  (apply variadic/adjust 4 (if extra? [3] nil)))

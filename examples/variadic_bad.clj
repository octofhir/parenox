(ns demo.variadic-bad
  (:require [demo.variadic :as variadic]))

(defn wrong-element {:t/sig '[:-> number?]} []
  (variadic/adjust 1 "bad"))

(defn missing-base {:t/sig '[:-> number?]} []
  (variadic/adjust))

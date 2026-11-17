(ns demo.apply-bad
  (:require [demo.arities.provider :as provider]
            [demo.variadic :as variadic]))

(defn wrong-element {:t/sig '[:-> number?]} []
  (apply variadic/adjust [4 "3"]))

(defn wrong-count {:t/sig '[:-> number?]} []
  (apply provider/describe ["A" "B"]))

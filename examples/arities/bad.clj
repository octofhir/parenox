(ns demo.arities.bad
  (:require [demo.arities.provider :as provider]))

(defn too-many {:t/sig '[:-> number?]} []
  (provider/count-offset 1 :done 7))

(defn wrong-phase {:t/sig '[:-> number?]} []
  (provider/count-offset 1 :other))

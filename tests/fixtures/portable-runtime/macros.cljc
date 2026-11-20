(ns portable.macro-probe
  (:require [portable.macros :as macros]))

(defn decorate [value]
  [:caller value])

(let [calls (atom 0)
      value 99]
  (println "cross-namespace macro")
  (prn [(macros/decorate-once (swap! calls inc)) @calls (decorate value)]))

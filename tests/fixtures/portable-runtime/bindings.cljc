(ns portable.binding-probe
  (:require [portable.dynamic :as dynamic]))

(let [observed (atom [(dynamic/mode)])]
  (binding [dynamic/*mode* :outer]
    (swap! observed conj (dynamic/mode))
    (try
      (binding [dynamic/*mode* :inner]
        (swap! observed conj (dynamic/mode))
        (throw (ex-info "exit binding" {})))
      (catch #?(:clj clojure.lang.ExceptionInfo :cljrs Exception) _
        (swap! observed conj (dynamic/mode)))
      (finally
        (swap! observed conj (dynamic/mode)))))
  (swap! observed conj (dynamic/mode))
  (println "dynamic unwind")
  (prn @observed))

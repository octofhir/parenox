(ns demo.list-accumulators-bad)

(defn invalid-tail {:t/sig '[:-> #{:ready}]} []
  (first (cons :ready 42)))

(ns demo.indices-bad)

(defn beyond-row
  {:t/sig '[:-> number?]}
  []
  (nth ["SKU-17" 120] 2))

(defn increment-explicit-nil
  {:t/sig '[:-> number?]}
  []
  (inc (get ["SKU-17" nil] 1 0)))

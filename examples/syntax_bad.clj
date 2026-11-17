(ns demo.syntax)

;; Deliberate mismatch; the following function remains visible in the CST.
[1 2)

(defn retained
  {:t/sig '[:-> string?]}
  []
  "still visible")

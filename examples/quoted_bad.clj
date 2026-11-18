(ns demo.quoted-bad)

(defn invalid-number {:t/sig '[:-> number?]} []
  (inc (nth '(1 "two") 1)))

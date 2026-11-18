(ns demo.numbers)

(defn invoice-total {:t/sig '[:-> number?]} []
  (+ 0x10 2r101 3/4 0.25M 7N))

(defn sample-id {:t/sig '[:-> number?]} []
  (+ 010 36rN 1.e2))

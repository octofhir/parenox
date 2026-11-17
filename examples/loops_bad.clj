(ns demo.loops-bad)

(defn invalid-later {:t/sig '[:-> number?]} []
  (loop [again? true value 0]
    (if again?
      (recur false "bad")
      (inc value))))

(defn wrong-arity {:t/sig '[:-> number?]} []
  (loop [i 0 total 0]
    (if false (recur 1) total)))

(defn non-tail {:t/sig '[:-> number?]} []
  (loop [i 0]
    (+ 1 (recur (inc i)))))

(defn fn-boundary {:t/sig '[:-> number?]} []
  (loop [i 0 total 0]
    ((fn [n] (recur n total)) i)))

(ns demo.reductions-bad)

(defn bad-element {:t/sig '[:-> number?]} []
  (reduce + 0 [1 "2"]))

(defn bad-transition {:t/sig '[:-> number?]} []
  (reduce (fn [acc _] (if (number? acc) "text" (inc acc)))
          0 [1 2]))

(defn bad-line-item {:t/sig '[:-> number?]} []
  (reduce (fn [total entry] (+ total (val entry)))
          0
          {:tax "unknown"}))

(ns demo.libspecs.provider)

(defn advance {:t/sig '[number? :-> number?]} [count]
  (+ count 2))

(defn spare {:t/sig '[:-> number?]} []
  10)

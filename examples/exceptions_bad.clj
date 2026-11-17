(ns demo.exceptions-bad)

(defn bad-exception {:t/sig '[:-> nil?]} []
  (throw "not an exception"))

(defn bad-cause {:t/sig '[:-> nil?]} []
  (throw (ex-info "Invalid cause" {} "not a cause")))

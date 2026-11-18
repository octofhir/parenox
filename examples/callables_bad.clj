(ns demo.callables-bad)

(defn missing-key {:t/sig '[:-> number?]} []
  ({:high 3}))

(defn extra-default {:t/sig '[:-> number?]} []
  (let [read-state :state]
    (read-state {:state 3} 1 2)))

(defn outside-vector {:t/sig '[:-> string?]} []
  (["only"] 3))

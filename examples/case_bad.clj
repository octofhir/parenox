(ns demo.case-bad)

(defn bad-flag-code {:t/sig '[boolean? :-> number?]} [flag]
  (case flag true (inc flag) false 0))

(defn absent-status {:t/sig '[:-> string?]} []
  (case :unknown
    :queued "pending"
    :done "finished"))

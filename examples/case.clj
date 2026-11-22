(ns demo.case)

(defn format-status {:t/sig '[#{:queued :running :done} :-> string?]} [status]
  (case status
    (:queued :running) "pending"
    :done "finished"))

(defn flag-code {:t/sig '[boolean? :-> number?]} [flag]
  (case flag true 1 false 0 "unreachable"))

(defn digit-label {:t/sig '[int? :-> string?]} [digit]
  (case digit
    (0 1) (nth ["zero" "one"] digit)
    "other"))

(defn published-label {:t/sig '[string? :-> string?]} [input]
  (loop [phase :queued label nil]
    (case phase
      :queued (recur :done input)
      :done label)))

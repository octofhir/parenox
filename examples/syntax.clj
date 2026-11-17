(ns demo.syntax)

;; The CST retains this comment and the discarded form below.
#_ (unfinished-work :not-analyzed)

(defn label
  {:t/sig '[string? :-> string?]}
  [name]
  (let [patient {:name name, :tags #{:active :visible}}]
    (str "Лиса: " (:name patient))))

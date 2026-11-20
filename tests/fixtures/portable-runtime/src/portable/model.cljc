(ns portable.model)

(defn summarize [base values]
  (let [adjust (fn [value] (+ base value))
        adjusted (map adjust (filter odd? values))
        sum (loop [remaining (seq adjusted) total 0]
              (if remaining
                (recur (next remaining) (+ total (first remaining)))
                total))
        original {:values values}
        updated (update original :values conj base)]
    [sum (vec adjusted) (:values original) (:values updated)
     (contains? (into #{} adjusted) 5)]))

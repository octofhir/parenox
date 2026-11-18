(ns demo.reductions)

(defn sum-values {:t/sig '[:-> number?]} []
  (reduce + [1 2 3]))

(defn total-in-map {:t/sig '[:-> number?]} []
  (:total
   (reduce (fn [state delta] (update state :total + delta))
           {:total 0}
           [1 2 3])))

(defn pick-collected {:t/sig '[:-> string?]} []
  (nth (reduce conj [] [1 "last"]) 1))

(defn append-labels {:t/sig '[:-> string?]} []
  (reduce (fn [acc label] (str acc "-" label))
          "a"
          (next [nil "b" "c"])))

(defn empty-count {:t/sig '[:-> number?]} []
  (reduce + (rest [1])))

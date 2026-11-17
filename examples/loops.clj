(ns demo.loops)

(defn triangular {:t/sig '[number? :-> number?]} [n]
  (loop [i 0 total 0]
    (if (= i n)
      total
      (recur (inc i) (+ total i)))))

(defn carried-label {:t/sig '[string? :-> string?]} [input]
  (loop [again? true label nil]
    (if again?
      (let [next-label (str input)]
        (recur false next-label))
      label)))

(defn nested-total {:t/sig '[number? :-> number?]} [n]
  (loop [i 0 total 0]
    (if (= i n)
      total
      (let [partial (loop [value i done? false]
                      (if done? value (recur (inc value) true)))]
        (recur (inc i) (+ total partial))))))

(defn last-value {:t/sig '[:-> number?]} []
  (loop [[value] [1] again? true]
    (if again? (recur [2] false) value)))

(defn local-function {:t/sig '[:-> number?]} []
  (loop [again? true total 0]
    (if again?
      (recur false ((fn [step?] (if step? (recur false) 7)) true))
      total)))

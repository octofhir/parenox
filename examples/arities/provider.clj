(ns demo.arities.provider)

(defn count-offset
  "Advance an offset once unless it is already done."
  ([offset] (count-offset offset :start))
  ([offset phase]
   (case phase
     :start (recur (inc offset) :done)
     :done offset))
  {:t/sig '[[number? #{:start :done} :-> number?]
            [number? :-> number?]]})

(defn describe
  {:t/sig '[[:-> string?] [string? :-> number?]]}
  ([] "ready")
  ([label] (count label)))

(ns demo)
(defn build [xs] (assoc (into {} xs) :done true))
(defn unary [xs] (vec (reverse xs)))
(defn commented [xs]
  (assoc ; between head and input
    (into {} xs) ; after input
    :done true))
(defn conditional [x] (if x :yes :no))
(defn empty-call [] (vector))
(defn quoted [xs] '(vec xs))
(defn threaded [xs] (-> (vec xs) (conj 0)))
(defn utf16 [xs] ["🧪" (vec (reverse xs))])
(defn shadow [-> xs] (vec xs))
(assoc, (into {} []), :done, true)
(defn discarded [xs] #_(vec xs) xs)

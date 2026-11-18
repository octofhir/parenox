(ns demo.tagged-maps
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]))

(s/def :note/kind #{:note})
(s/def :note/value string?)
(s/def :counter/kind #{:counter})
(s/def :counter/value number?)
(s/def ::note (s/keys :req-un [:note/kind :note/value]))
(s/def ::counter (s/keys :req-un [:counter/kind :counter/value]))
(s/def ::event (s/or :as-note ::note :as-counter ::counter))
(s/def ::item ::event)
(s/def ::envelope (s/keys :req-un [::item]))

(defn label {:t/sig '[::event :-> string?]} [event]
  (if (= :note (:kind event))
    (str/upper-case (:value event))
    (str (inc (:value event)))))

(defn dispatch {:t/sig '[::event :-> string?]} [event]
  (let [kind (get event :kind)]
    (case kind
      :counter (str (inc (:value event)))
      (str/upper-case (:value event)))))

(defn nested-label {:t/sig '[::envelope :-> string?]} [envelope]
  (let [item (:item envelope)]
    (cond
      (= (get-in envelope [:item :kind]) :note) (str/upper-case (:value item))
      :else (str (inc (:value item))))))

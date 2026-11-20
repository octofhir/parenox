(ns portable.async-timeout-port
  (:require [clojure.core.async :as async]))

(defn timeout-port [ms]
  #?(:clj (async/timeout ms)
     :cljrs (let [port (async/chan)
                  timer (async/timeout ms)]
              (async/go
                (await timer)
                (async/close! port))
              port)))

(let [data (async/chan 1)
      timer (timeout-port 5000)]
  (assert (async/offer! data :data))
  (let [[value port] (async/alts!! [data timer] :priority true)]
    (assert (identical? data port))
    (assert (= :data value))
    (prn [:data-wins value])))

(let [empty (async/chan)
      timer (timeout-port 20)
      [value port] (async/alts!! [empty timer] :priority true)]
  (assert (identical? timer port))
  (assert (nil? value))
  (assert (nil? (async/<!! timer)))
  ;; A pending take left by selection would consume this offer.
  (assert (not (async/offer! empty :lost)))
  (async/go
    (async/>! empty :after-timeout)
    (async/close! empty))
  (let [value-after (async/<!! empty)]
    (assert (= :after-timeout value-after))
    (assert (nil? (async/<!! empty)))
    (prn [:timer-wins value value-after])))

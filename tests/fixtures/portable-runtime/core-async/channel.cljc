(ns portable.async-channel
  (:require [clojure.core.async :as async]))

(let [channel (async/chan)
      payload {:answer 42}]
  (async/go
    (async/>! channel payload)
    (async/close! channel))
  (let [received [(async/<!! channel) (async/<!! channel)]]
    (assert (= [payload nil] received))
    (prn received)))

(ns portable.async-timeout
  (:require [clojure.core.async :as async]))

(let [channel (async/chan)
      timeout (async/timeout 5000)
      payload {:answer 42}]
  (async/go
    (async/>! channel payload)
    (async/close! channel))
  (let [[value value-port] (async/alts!! [channel timeout] :priority true)
        [closed close-port] (async/alts!! [channel timeout] :priority true)]
    (when-not (and (identical? channel value-port)
                   (identical? channel close-port)
                   (= payload value)
                   (nil? closed))
      (throw (ex-info "Expected one channel value followed by closure"
                      {:value value :closed closed})))
    (prn [value closed])))

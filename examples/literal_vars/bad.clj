(ns demo.literal-vars.bad
  (:require [demo.literal-vars.config :as config]))

(defn private-fallback {:t/sig '[:-> string?]} []
  config/fallback)

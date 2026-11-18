(ns demo.routing
  (:require [clojure.spec.alpha :as s]))

(defn status
  {:t/sig '[(t/map {:opt {:route (s/nilable keyword?)} :extra :open})
            :-> #{:found :missing}]}
  [request]
  (if (some? (:route request)) :found :missing))

(defn platform
  {:t/sig '[:-> #{:jvm :browser}]}
  []
  #?(:clj :jvm :cljs :browser))

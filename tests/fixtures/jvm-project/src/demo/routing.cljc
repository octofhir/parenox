(ns demo.routing)

(defn status [request]
  (if (some? (:route request)) :found :missing))

(defn platform []
  #?(:clj :jvm :cljs :browser))

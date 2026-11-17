(ns demo.libspecs.bad
  (:require [demo.libspecs.provider :refer [advance] :rename {advance bump}]))

(defn broken-counter {:t/sig '[:-> number?]} []
  (bump "not a number"))

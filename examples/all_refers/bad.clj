(ns demo.all-refers.bad
  (:refer-clojure :exclude [inc])
  (:require [demo.all-refers.library :refer :all :exclude [skipped]]))

(defn uses-excluded {:t/sig '[:-> number?]} []
  (skipped))

(defn uses-private {:t/sig '[:-> string?]} []
  (hidden))

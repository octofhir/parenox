(ns demo.all-refers.app
  (:refer-clojure :exclude [inc])
  (:require [demo.all-refers.library :as lib :refer :all :exclude [skipped]
             :rename {label caption}]))

(defn render {:t/sig '[:-> string?]} []
  (caption))

(defn window-size {:t/sig '[:-> number?]} []
  (inc limit))

(defn fallback-size {:t/sig '[:-> number?]} []
  (lib/skipped))

(ns demo.libspecs.app
  (:require clojure.string
            [demo.libspecs.provider :refer (advance spare)
             :exclude [spare] :rename {advance bump}]))

(defn bump-counter {:t/sig '[number? :-> number?]} [count]
  (inc (bump count)))

(defn clean-label {:t/sig '[string? :-> string?]} [label]
  (clojure.string/trim label))

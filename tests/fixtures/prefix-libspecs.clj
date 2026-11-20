(ns prefix-libspecs-fixture
  (:require (prefix-libspecs [values :as values :refer [answer]])
            [prefix-libspecs values
             (tools :as tools)
             [reader-only :as-alias reader-only]])
  (:use (prefix-libspecs [tools :only [tool]])))

(assert (= [42 42 :portable-tool]
           [(answer 41) (values/answer 41) tool]))
(assert (= tools/tool tool))
(assert (= :prefix-libspecs.reader-only/key ::reader-only/key))
(assert (= {:prefix-libspecs.reader-only/key :value}
           #::reader-only{:key :value}))
(assert (= 'prefix-libspecs.reader-only
           (ns-name (get (ns-aliases *ns*) 'reader-only))))
(assert (not (contains? (loaded-libs) 'prefix-libspecs.reader-only)))

(prn :prefix-libspecs-ok)

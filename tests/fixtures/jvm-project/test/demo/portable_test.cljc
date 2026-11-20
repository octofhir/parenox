(ns demo.portable-test
  (:require [clojure.test :refer [deftest is]]))

(defn runtime []
  #?(:cljrs :native :clj :jvm :default :other))

#?(:cljrs (deftest selects-native-branch (is (= :native (runtime))))
   :clj (deftest selects-jvm-branch (is (= :jvm (runtime)))))

(deftest map-presence-and-truthiness
  (let [item {:label nil :enabled false}]
    (is (contains? item :label))
    (is (nil? (get item :label "absent")))
    (is (= "absent" (get {} :label "absent")))
    (is (= :off (if (:enabled item) :on :off)))
    (is (= 0 (or 0 :fallback)))))

(deftest dereferences-an-updated-map
  (let [state (atom {:seen #{}})]
    (swap! state update :seen conj :ready)
    (is (= {:seen #{:ready}} @state))))

(def ^:dynamic *multiplier* 1)

(defmacro scaled [value]
  `(* *multiplier* ~value))

(deftest macros-are-evaluated-with-the-current-binding
  (is (= 12 (binding [*multiplier* 3] (scaled 4))))
  (is (= 4 (scaled 4))))

(deftest handles-exception-data
  (is (= {:code :failure}
         (try
           (throw (ex-info "portable exception" {:code :failure}))
           #?(:clj (catch clojure.lang.ExceptionInfo error (ex-data error))
              :cljrs (catch Exception error (ex-data error)))))))

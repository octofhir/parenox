(ns checked-deftest
  (:require [clojure.string :as str]
            [clojure.test :as t :refer [deftest is testing]]))

(deftest checked-predicates
  (testing "outer"
    (is (= 42 (+ 40 2)))
    (testing "inner"
      (is (nil? (get {:present nil} :present)) "present nil")
      (is (some? false) "false is present")
      (is (true? (not false)) "true predicate")
      (is (false? (not true)) "false predicate")
      (is (not false) "plain not")
      (is true "literal true")
      (is false "literal false reports failure"))
    (is (= :after :after) "outer context restored")))

(deftest checked-operand-error
  (testing "operand"
    (is (= "" (str/trim 42)) "trim requires text")))

(let [events (atom [])]
  (binding [t/*testing-contexts* ()
            t/report (fn [event]
                       (when (#{:pass :fail :error} (:type event))
                         (swap! events conj
                                (assoc event :contexts (vec (reverse t/*testing-contexts*))))))]
    (t/test-var #'checked-predicates)
    (t/test-var #'checked-operand-error))
  (assert
   (= [[:pass ["outer"] nil '(= 42 (+ 40 2)) '(= 42 42)]
       [:pass ["outer" "inner"] "present nil" '(nil? (get {:present nil} :present)) '(nil? nil)]
       [:pass ["outer" "inner"] "false is present" '(some? false) '(some? false)]
       [:pass ["outer" "inner"] "true predicate" '(true? (not false)) '(true? true)]
       [:pass ["outer" "inner"] "false predicate" '(false? (not true)) '(false? false)]
       [:pass ["outer" "inner"] "plain not" '(not false) '(not false)]
       [:pass ["outer" "inner"] "literal true" true true]
       [:fail ["outer" "inner"] "literal false reports failure" false false]
       [:pass ["outer"] "outer context restored" '(= :after :after) '(= :after :after)]
       [:error ["operand"] "trim requires text" '(= "" (str/trim 42)) ClassCastException]]
      (mapv (fn [{:keys [type contexts message expected actual]}]
              [type contexts message expected (if (= :error type) (class actual) actual)])
            @events)))
  (prn (into {:clojure (clojure-version)} (frequencies (map :type @events)))))

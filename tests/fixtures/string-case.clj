(ns demo.string-case
  (:require [clojure.string :as str])
  (:import [clojure.lang ArityException]
           [java.util Locale]))

(defn failure [attempt]
  (try
    (attempt)
    nil
    (catch Throwable error error)))

(let [previous-locale (Locale/getDefault)
      case-functions [str/upper-case str/lower-case str/capitalize]]
  (try
    (Locale/setDefault Locale/ROOT)
    (doseq [[input expected]
            [[42 ["42" "42" "42"]]
             [7N ["7" "7" "7"]]
             [false ["FALSE" "false" "False"]]
             [:demo/value [":DEMO/VALUE" ":demo/value" ":demo/value"]]
             [:clojure.spec.alpha/invalid [":CLOJURE.SPEC.ALPHA/INVALID"
                                           ":clojure.spec.alpha/invalid"
                                           ":clojure.spec.alpha/invalid"]]
             [\q ["Q" "q" "Q"]]
             [{:hello 1} ["{:HELLO 1}" "{:hello 1}" "{:hello 1}"]]
             [[false "a"] ["[FALSE \"A\"]" "[false \"a\"]" "[false \"a\"]"]]
             [(StringBuilder. "mIx") ["MIX" "mix" "Mix"]]]]
      (assert (= expected [(str/upper-case input) (str/lower-case input) (str/capitalize input)]))
      (assert (= expected (mapv #(apply % [input]) case-functions))))
    (assert (= ["MIX" "FALSE" "7"] (mapv str/upper-case ["mIx" false 7])))

    (let [thrown (ex-info "toString failed" {:reason :to-string})
          throwing (reify Object (toString [_] (throw thrown)))
          returning-nil (reify Object (toString [_] nil))]
      (doseq [op case-functions]
        (assert (instance? NullPointerException (failure #(op nil))))
        (assert (instance? NullPointerException (failure #(op returning-nil))))
        (assert (identical? thrown (failure #(op throwing))))
        (assert (instance? ArityException (failure #(op))))
        (assert (instance? ArityException (failure #(op "a" "b"))))))

    (doseq [[op expected] [[str/trim "x"]
                           [str/triml "x \r\n"]
                           [str/trimr " x"]
                           [str/trim-newline " x "]]]
      (assert (= expected (op (StringBuilder. " x \r\n"))))
      (assert (instance? NullPointerException (failure #(op nil))))
      (doseq [value [42 false :value]]
        (assert (instance? ClassCastException (failure #(op value))))))
    (assert (str/blank? nil))
    (assert (str/blank? false))
    (assert (str/blank? (StringBuilder. " \t")))
    (assert (not (str/blank? (StringBuilder. " x"))))
    (assert (instance? ClassCastException (failure #(str/blank? true))))
    (assert (instance? ClassCastException (failure #(str/blank? 42))))
    (assert (str/blank? "\u2003"))
    (assert (not (str/blank? "\u00a0")))

    (assert (= "STRASSE" (str/upper-case "straße")))
    (Locale/setDefault (Locale/forLanguageTag "tr-TR"))
    (assert (= "İ" (str/upper-case "i")))
    (assert (= "ı" (str/lower-case "I")))
    (finally (Locale/setDefault previous-locale))))

(prn :string-case-ok (clojure-version))

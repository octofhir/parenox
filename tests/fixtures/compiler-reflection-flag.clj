(ns demo.compiler-reflection-flag
  (:require [clojure.core :as core]
            [clojure.string :as str])
  (:import [java.io StringWriter]))

(defn compile-with-warnings [form]
  (let [warnings (StringWriter.)
        f (binding [*err* warnings] (eval form))]
    [f (str warnings)]))

(let [before *warn-on-reflection*
      form '(fn [input] (.length input))]
  (binding [*warn-on-reflection* false]
    (set! *warn-on-reflection* true)
    (assert (true? clojure.core/*warn-on-reflection*))
    (let [[warned warning] (compile-with-warnings form)]
      (assert (str/includes? warning "Reflection warning"))

      (set! clojure.core/*warn-on-reflection* false)
      (assert (false? *warn-on-reflection*))
      (let [[quiet quiet-output] (compile-with-warnings form)]
        (assert (not (str/includes? quiet-output "Reflection warning")))

        (set! core/*warn-on-reflection* true)
        (let [[aliased alias-warning] (compile-with-warnings form)
              input (StringBuilder. "abc")]
          (assert (str/includes? alias-warning "Reflection warning"))
          (assert (= 3 (warned input) (quiet input) (aliased input)))))))
  (assert (identical? before *warn-on-reflection*)))

(let [before *warn-on-reflection*
      name (gensym "demo.reflection-shadow-")
      shadow (create-ns name)]
  (try
    (binding [*ns* shadow
              clojure.core/*warn-on-reflection* false]
      (refer 'clojure.core :exclude '[*warn-on-reflection*])
      (eval '(def ^:dynamic *warn-on-reflection* :root))
      (let [values (eval '(binding [*warn-on-reflection* :before]
                           [(set! *warn-on-reflection* true)
                            *warn-on-reflection*
                            clojure.core/*warn-on-reflection*]))]
        (assert (= [true true false] values)))
      (assert (= :root (var-get (ns-resolve shadow '*warn-on-reflection*))))
      (assert (false? clojure.core/*warn-on-reflection*)))
    (finally (remove-ns name)))
  (assert (identical? before *warn-on-reflection*)))

(prn :compiler-reflection-flag-ok (clojure-version))

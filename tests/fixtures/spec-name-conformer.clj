(ns spec-name-conformer
  (:require [clojure.spec.alpha :as s])
  (:import [clojure.lang Named]))

;; JVM reference: clojure -M tests/fixtures/spec-name-conformer.clj

(s/def ::converted-name (s/conformer name))
(s/def ::label ::converted-name)
(s/def ::record (s/keys :req-un [::label]))
(s/def ::keyword-name (s/and keyword? (s/conformer name)))

(doseq [text [(String. "") (String. "/") (String. "topic/a/b")]]
  (assert (identical? text (name text)))
  (assert (identical? text (s/conform ::label text))))

(doseq [[value expected] [[:plain "plain"]
                          [:/ "/"]
                          [:topic/a/b "a/b"]
                          ['plain "plain"]
                          ['topic/a/b "a/b"]
                          [::s/invalid "invalid"]]]
  (assert (= expected (name value)))
  (assert (= expected (s/conform ::converted-name value)))
  (assert (s/valid? ::label value))
  (assert (= {:label expected} (s/conform ::record {:label value}))))

;; A preceding spec can propagate the reserved value before name is called.
(assert (= "a/b" (s/conform ::keyword-name :topic/a/b)))
(assert (= ::s/invalid (s/conform ::keyword-name ::s/invalid)))

(doseq [value [(symbol nil nil)
               (reify Named
                 (getName [_] nil)
                 (getNamespace [_] "custom"))]]
  (assert (nil? (name value)))
  (assert (nil? (s/conform ::label value)))
  (assert (s/valid? ::label value))
  (assert (= {:label nil} (s/conform ::record {:label value}))))

(let [text (String. "custom/name")
      named (reify Named
              (getName [_] text)
              (getNamespace [_] "custom"))]
  (assert (identical? text (name named)))
  (assert (identical? text (s/conform ::label named))))

(let [number-name (proxy [Number Named] []
                    (intValue [] 7)
                    (longValue [] 7)
                    (floatValue [] 7.0)
                    (doubleValue [] 7.0)
                    (getName [] "number/name")
                    (getNamespace [] "custom"))]
  (assert (number? number-name))
  (assert (not (int? number-name)))
  (assert (= "number/name" (name number-name)))
  (assert (= "number/name" (s/conform ::label number-name))))

(defn exception [f]
  (try (f) nil (catch Exception e e)))

(doseq [[value expected] [[nil NullPointerException]
                          [42 ClassCastException]
                          [(int 42) ClassCastException]
                          [false ClassCastException]]
        f [name #(s/conform ::label %) #(s/valid? ::label %)]]
  (assert (= expected (class (exception #(f value))))))

(let [failure (ex-info "custom name failed" {:source :named})
      named (reify Named
              (getName [_] (throw failure))
              (getNamespace [_] nil))]
  (assert (identical? failure (exception #(name named))))
  (assert (identical? failure (exception #(s/conform ::label named))))
  (assert (identical? failure (exception #(s/valid? ::label named)))))

(let [error (exception #(s/unform ::label "plain"))]
  (assert (= IllegalStateException (class error)))
  (assert (= "no unform fn for conformer" (ex-message error))))

(prn {:clojure (clojure-version) :spec-name-conformer :ok})

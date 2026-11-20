(ns spec-text-document-sync
  (:require [clojure.spec.alpha :as s]))

;; JVM reference: clojure -M tests/fixtures/spec-text-document-sync.clj
;; Keep the upstream spelling because the conformer reads this var at runtime.
(def text-docyment-sync-kind
  {:none 0
   :full 1
   :incremental 2})
(s/def :server-capabilities/text-document-sync
  (s/conformer (fn [element]
                 {:open-close true
                  :change (or (get text-docyment-sync-kind element)
                              (get text-docyment-sync-kind :full))
                  :save {:include-text true}})))
(s/def ::server-capabilities
  (s/keys :opt-un [:server-capabilities/text-document-sync]))

(def provider :server-capabilities/text-document-sync)

(defn assert-provider [element change]
  (let [expected {:open-close true :change change :save {:include-text true}}
        capabilities {:text-document-sync element}]
    (assert (= expected (s/conform provider element)))
    (assert (true? (s/valid? provider element)))
    (assert (= {:text-document-sync expected}
               (s/conform ::server-capabilities capabilities)))
    (assert (true? (s/valid? ::server-capabilities capabilities)))))

(doseq [[element change] [[:none 0] [:full 1] [:incremental 2]]]
  (assert-provider element change))
(doseq [element [nil false ::s/invalid "none" 'none :other/none {} [] (Object.)]]
  (assert-provider element 1))

(let [zero-key (proxy [Object] []
                 (equals [other] (identical? other :none)))
      hash-key (proxy [Object] []
                 (hashCode [] (throw (IllegalStateException. "lookup hashed the key"))))]
  (assert-provider zero-key 0)
  (assert-provider hash-key 1))

(with-redefs [text-docyment-sync-kind
              {:none false :incremental nil :selected 0 :full :fallback}]
  (assert-provider :none :fallback)
  (assert-provider :incremental :fallback)
  (assert-provider :selected 0)
  (assert-provider :unknown :fallback))

(doseq [fallback [nil false 0 ::s/invalid]]
  (with-redefs [text-docyment-sync-kind {:full fallback}]
    (assert-provider :unknown fallback)))
(with-redefs [text-docyment-sync-kind {}]
  (assert-provider :unknown nil))

(let [table (java.util.HashMap. {:full 7})]
  (with-redefs [text-docyment-sync-kind table]
    (assert-provider :incremental 7)
    (.put table :incremental 0)
    (assert-provider :incremental 0)
    (.put table :incremental false)
    (assert-provider :incremental 7)))

(let [table (reify clojure.lang.ILookup
              (valAt [_ _]
                (alter-var-root #'text-docyment-sync-kind (constantly {:full 9}))
                false)
              (valAt [_ _ fallback] fallback))]
  (with-redefs [text-docyment-sync-kind table]
    (assert (= 9 (:change (s/conform provider :first-lookup))))))
(assert-provider :incremental 2)

(let [failure (ex-info "lookup failed" {})
      key (proxy [Object] []
            (equals [_] (throw failure)))
      table (reify clojure.lang.ILookup
              (valAt [_ key]
                (case key
                  :found 0
                  :retry nil
                  (throw failure)))
              (valAt [_ _ _] (throw failure)))
      invokes [#(s/conform provider %) #(s/valid? provider %)
               #(s/conform ::server-capabilities {:text-document-sync %})
               #(s/valid? ::server-capabilities {:text-document-sync %})]]
  (doseq [invoke invokes]
    (assert (identical? failure (try (invoke key) nil (catch Exception error error)))))
  (with-redefs [text-docyment-sync-kind table]
    (assert-provider :found 0)
    (doseq [invoke invokes
            input [:missing :retry]]
      (assert (identical? failure (try (invoke input) nil (catch Exception error error)))))))

(prn {:clojure (clojure-version) :spec-text-document-sync :ok})

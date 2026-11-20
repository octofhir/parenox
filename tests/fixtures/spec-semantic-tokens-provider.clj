(ns spec-semantic-tokens-provider
  (:require [clojure.spec.alpha :as s]))

;; JVM reference: clojure -M tests/fixtures/spec-semantic-tokens-provider.clj
;; The conformer and capability key are from lsp4clj 2.0.1.
(s/def :server-capabilities/semantic-tokens-provider
  (s/conformer #(when (and (:token-types %)
                           (:token-modifiers %))
                  {:legend {:token-types (:token-types %)
                            :token-modifiers (:token-modifiers %)}
                   :range (:range %)
                   :full (boolean (get % :full false))})))
(s/def ::server-capabilities
  (s/keys :opt-un [:server-capabilities/semantic-tokens-provider]))

(def provider :server-capabilities/semantic-tokens-provider)

(defn assert-provider [element expected]
  (let [capabilities {:semantic-tokens-provider element}]
    (assert (= expected (s/conform provider element)))
    (assert (true? (s/valid? provider element)))
    (assert (= {:semantic-tokens-provider expected}
               (s/conform ::server-capabilities capabilities)))
    (assert (true? (s/valid? ::server-capabilities capabilities)))))

(doseq [element [nil false true ::s/invalid :enabled 0 [] {} (find {:entry :value} :entry)
                 {:token-types []} {:token-modifiers []}
                 {:token-types false :token-modifiers []}
                 {:token-types [] :token-modifiers nil}]]
  (assert-provider element nil))

(let [input {:token-types [] :token-modifiers {}}
      base {:legend input :range nil :full false}]
  (assert-provider input base)
  (assert-provider (java.util.HashMap. input) base)
  (assert-provider (assoc input :range ::s/invalid :full false)
                   (assoc base :range ::s/invalid))
  (assert-provider (assoc input :range false :full nil) (assoc base :range false))
  (doseq [full [true 0 "" [] {} ::s/invalid]]
    (assert-provider (assoc input :full full) (assoc base :full true))))

(let [input {:token-types ::s/invalid :token-modifiers false}
      accepted (assoc input :token-modifiers 0)]
  (assert-provider input nil)
  (assert-provider accepted {:legend accepted :range nil :full false}))

(assert-provider (sorted-map-by (constantly 0) :only :match)
                 {:legend {:token-types :match :token-modifiers :match}
                  :range :match
                  :full true})
(doseq [invoke [#(s/conform provider %) #(s/valid? provider %)]]
  (assert (try
            (invoke (sorted-map "only" []))
            false
            (catch ClassCastException _ true))))

(let [failure (ex-info "lookup failed" {})
      guard-stops (reify clojure.lang.ILookup
                    (valAt [_ key] (if (= key :token-types) false (throw failure)))
                    (valAt [_ _ _] (throw failure)))
      guard-throws (reify clojure.lang.ILookup
                     (valAt [_ key] (if (= key :token-types) [] (throw failure)))
                     (valAt [_ _ _] (throw failure)))
      full-throws (reify clojure.lang.ILookup
                    (valAt [_ key] (get {:token-types [] :token-modifiers []} key))
                    (valAt [_ _ _] (throw failure)))]
  (assert-provider guard-stops nil)
  (doseq [lookup [guard-throws full-throws]
          invoke [#(s/conform provider %) #(s/valid? provider %)
                  #(s/conform ::server-capabilities {:semantic-tokens-provider %})
                  #(s/valid? ::server-capabilities {:semantic-tokens-provider %})]]
    (assert (identical? failure (try (invoke lookup) nil (catch Exception error error))))))

(let [calls (atom [])
      lookup (reify clojure.lang.ILookup
               (valAt [_ key]
                 (let [seen (some #{key} @calls)]
                   (swap! calls conj key)
                   (case key
                     :token-types (when-not seen [])
                     :token-modifiers (if seen false [])
                     :range ::s/invalid)))
               (valAt [_ key default-value]
                 (swap! calls conj [key default-value])
                 ::s/invalid))
      expected-calls [:token-types :token-modifiers :token-types :token-modifiers :range [:full false]]]
  (assert (= {:legend {:token-types nil :token-modifiers false} :range ::s/invalid :full true}
             (s/conform provider lookup)))
  (assert (= expected-calls @calls))
  (reset! calls [])
  (assert (true? (s/valid? provider lookup)))
  (assert (= expected-calls @calls)))

(prn {:clojure (clojure-version) :spec-semantic-tokens-provider :ok})

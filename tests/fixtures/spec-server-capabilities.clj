(ns spec-server-capabilities
  (:require [clojure.spec.alpha :as s]))

;; JVM reference: clojure -M tests/fixtures/spec-server-capabilities.clj
(defn outcome [f spec input]
  (try (f spec input) (catch Exception error error)))

(doseq [f [s/conform s/valid?]]
  (let [result (outcome f ::server-capabilities {})]
    (assert (instance? Exception result))
    (assert (= (str "Unable to resolve spec: " ::server-capabilities)
               (ex-message result)))))

;; lsp4clj 2.0.1 declarations.
(s/def :server-capabilities/signature-help-provider
  (s/conformer #(cond (vector? %) {:trigger-characters %}
                      (map? %) %
                      :else {:trigger-characters %})))
(s/def :server-capabilities/code-action-provider
  (s/conformer #(when (vector? %) {:code-action-kinds %})))
(s/def :server-capabilities/execute-command-provider
  (s/conformer #(when (vector? %) {:commands %})))
(s/def :server-capabilities/code-lens-provider
  (s/conformer (fn [element] {:resolve-provider element})))
(s/def :server-capabilities/rename-provider
  (s/conformer (fn [element] {:prepare-provider element})))
(s/def :server-capabilities/semantic-tokens-provider
  (s/conformer #(when (and (:token-types %)
                           (:token-modifiers %))
                  {:legend {:token-types (:token-types %)
                            :token-modifiers (:token-modifiers %)}
                   :range (:range %)
                   :full (boolean (get % :full false))})))
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
  (s/keys :opt-un [:server-capabilities/code-action-provider
                   :server-capabilities/code-lens-provider
                   :server-capabilities/execute-command-provider
                   :server-capabilities/rename-provider
                   :server-capabilities/semantic-tokens-provider
                   :server-capabilities/signature-help-provider
                   :server-capabilities/text-document-sync]))

;; Bundle assertions.
(defn assert-capabilities [input expected]
  (assert (= expected (s/conform ::server-capabilities input)))
  (assert (true? (s/valid? ::server-capabilities input))))

(doseq [input [nil false ::s/invalid [] (find {:entry 7} :entry) (java.util.HashMap.)]]
  (assert (= ::s/invalid (s/conform ::server-capabilities input)))
  (assert (false? (s/valid? ::server-capabilities input))))

(doseq [input [{} {:unmodeled false :unregistered/status ::s/invalid}]]
  (assert (identical? input (s/conform ::server-capabilities input)))
  (assert (true? (s/valid? ::server-capabilities input))))

(let [signature {:trigger-characters ["."]}
      legend {:token-types [:type] :token-modifiers []}
      input {:code-action-provider [:rename]
             :execute-command-provider ["command"]
             :code-lens-provider ::s/invalid
             :rename-provider false
             :signature-help-provider signature
             :semantic-tokens-provider (assoc legend :range ::s/invalid :full 0)
             :text-document-sync :incremental}
      result (s/conform ::server-capabilities input)]
  (assert (= {:code-action-provider {:code-action-kinds [:rename]}
              :execute-command-provider {:commands ["command"]}
              :code-lens-provider {:resolve-provider ::s/invalid}
              :rename-provider {:prepare-provider false}
              :signature-help-provider signature
              :semantic-tokens-provider {:legend legend :range ::s/invalid :full true}
              :text-document-sync {:open-close true :change 2 :save {:include-text true}}}
             result))
  (assert (identical? signature (:signature-help-provider result)))
  (assert (true? (s/valid? ::server-capabilities input))))

(doseq [element [nil false]]
  (assert-capabilities
   {:code-action-provider element
    :execute-command-provider element
    :code-lens-provider element
    :rename-provider element
    :semantic-tokens-provider element
    :signature-help-provider element
    :text-document-sync element}
   {:code-action-provider nil
    :execute-command-provider nil
    :code-lens-provider {:resolve-provider element}
    :rename-provider {:prepare-provider element}
    :semantic-tokens-provider nil
    :signature-help-provider {:trigger-characters element}
    :text-document-sync {:open-close true :change 1 :save {:include-text true}}}))

(assert-capabilities {:code-lens-provider false :server-capabilities/code-lens-provider nil}
                     {:code-lens-provider {:resolve-provider false}
                      :server-capabilities/code-lens-provider {:resolve-provider nil}})

(defrecord Capabilities [code-lens-provider])
(let [result (s/conform ::server-capabilities (->Capabilities false))]
  (assert (instance? Capabilities result))
  (assert (= (->Capabilities {:resolve-provider false}) result)))

(let [input {:limit 8N :extra/limit 7N}]
  (assert (identical? input (s/conform ::server-capabilities input)))
  (s/def :extra/limit (s/and integer? (s/conformer int)))
  (let [result (s/conform ::server-capabilities input)]
    (assert (= {:limit 8N :extra/limit 7} result))
    (assert (identical? (:limit input) (:limit result)))
    (assert (= Integer (class (:extra/limit result))))
    (assert (true? (s/valid? ::server-capabilities input))))
  (let [invalid (assoc input :extra/limit "bad")]
    (assert (= ::s/invalid (s/conform ::server-capabilities invalid)))
    (assert (false? (s/valid? ::server-capabilities invalid))))
  (doseq [f [s/conform s/valid?]]
    (let [overflow (assoc input :extra/limit 2147483648)]
      (assert (instance? ArithmeticException (outcome f ::server-capabilities overflow)))))
  (s/def :extra/limit nil)
  (assert (identical? input (s/conform ::server-capabilities input))))

(let [input {:code-lens-provider ::s/invalid}]
  (s/def :server-capabilities/code-lens-provider nil)
  (assert (identical? input (s/conform ::server-capabilities input)))
  (assert (true? (s/valid? ::server-capabilities input)))
  (s/def :server-capabilities/code-lens-provider
    (s/conformer (fn [element] {:resolve-provider element})))
  (assert-capabilities input {:code-lens-provider {:resolve-provider ::s/invalid}}))

(s/def :extra/reject string?)
(let [failure (ex-info "lookup failed" {})
      lookup (reify clojure.lang.ILookup
               (valAt [_ _] (throw failure))
               (valAt [_ _ _] (throw failure)))
      invalid-first (array-map :extra/reject false :semantic-tokens-provider lookup)
      throw-first (array-map :semantic-tokens-provider lookup :extra/reject false)]
  (assert (= ::s/invalid (s/conform ::server-capabilities invalid-first)))
  (assert (false? (s/valid? ::server-capabilities invalid-first)))
  (doseq [f [s/conform s/valid?]]
    (assert (identical? failure (outcome f ::server-capabilities throw-first)))))
(s/def :extra/reject nil)

(prn {:clojure (clojure-version) :spec-server-capabilities :ok})

(ns demo.folding-ranges
  (:require [clojure.spec.alpha :as s]))

;; The relevant lsp4clj 2.0.1 declarations, runnable without the library.
(s/def :folding-range/kind keyword?)

(s/def ::folding-range
  (s/keys :req-un [::start-line ::end-line]
          :opt-un [::start-character ::end-character :folding-range/kind ::collapsed-text]))

(s/def ::folding-ranges (s/coll-of ::folding-range))

;; JVM-only assertions.
(def folding-range {:start-line 1N :end-line 3})

(assert (s/valid? ::folding-range folding-range))
(assert (identical? folding-range (s/conform ::folding-range folding-range)))

(doseq [invalid [nil
                 (dissoc folding-range :start-line)
                 (dissoc folding-range :end-line)
                 {::start-line 1 :end-line 3}]]
  (assert (not (s/valid? ::folding-range invalid)))
  (assert (= ::s/invalid (s/conform ::folding-range invalid))))

(doseq [unregistered [nil false ::s/invalid]]
  (let [present {:start-line unregistered :end-line unregistered
                 :start-character unregistered :end-character unregistered
                 :collapsed-text unregistered}]
    (assert (s/valid? ::folding-range present))
    (assert (identical? present (s/conform ::folding-range present)))
    (assert (= [present] (s/conform ::folding-ranges [present])))))

(doseq [keyword-value [:region :custom]]
  (let [with-kind (assoc folding-range :kind keyword-value)]
    (assert (s/valid? ::folding-range with-kind))
    (assert (identical? with-kind (s/conform ::folding-range with-kind)))))

(doseq [bad-kind [nil false "region" ::s/invalid]]
  (let [invalid (assoc folding-range :kind bad-kind)]
    (assert (not (s/valid? ::folding-range invalid)))
    (assert (= ::s/invalid (s/conform ::folding-range invalid)))
    (assert (= ::s/invalid (s/conform ::folding-ranges [invalid])))))

(doseq [empty-input [[] () #{} {}]]
  (let [result (s/conform ::folding-ranges empty-input)]
    (assert (= empty-input result))
    (assert (= (type empty-input) (type result)))))

(doseq [invalid [nil ::s/invalid [::s/invalid] {:entry folding-range}
                 (clojure.lang.MapEntry/create :entry folding-range)]]
  (assert (= ::s/invalid (s/conform ::folding-ranges invalid))))

(let [entry (clojure.lang.MapEntry/create folding-range folding-range)]
  (assert (identical? entry (s/conform ::folding-ranges entry)))

  ;; Map specs consult the registry at each conformance call.
  (s/def ::start-line (s/and integer? (s/conformer int)))
  (s/def ::collapsed-text string?)

  (let [converted (s/conform ::folding-range folding-range)
        result (s/conform ::folding-ranges entry)]
    (assert (= folding-range converted))
    (assert (= Integer (type (:start-line converted))))
    (assert (= :retained (:end-line (s/conform ::folding-range (assoc folding-range :end-line :retained)))))
    (assert (= [converted converted] result))
    (assert (vector? result))
    (assert (not (map-entry? result)))
    (assert (every? #(= Integer (type (:start-line %))) result)))
  (doseq [invalid [(assoc folding-range :start-line nil)
                   (assoc folding-range :start-line ::s/invalid)
                   (assoc folding-range :collapsed-text false)]]
    (assert (= ::s/invalid (s/conform ::folding-range invalid)))))

(s/def ::kind-alias :folding-range/kind)
(s/def ::lift-marker
  (s/conformer (fn [value] (if (= value ::s/invalid) :lifted value))))
(s/def ::keyword-first (s/and keyword? ::lift-marker))
(s/def ::lift-first (s/and ::lift-marker keyword?))
(s/def ::keyword-or-lift (s/or :keyword keyword? :lift ::lift-marker))
(s/def ::optional-lifter (s/keys :opt-un [::lift-marker]))

(assert (keyword? ::s/invalid))
(doseq [spec [:folding-range/kind ::kind-alias ::keyword-first
              (s/and keyword? any?) (s/and any? keyword?)]]
  (assert (s/valid? spec :region))
  (assert (= :region (s/conform spec :region)))
  (assert (not (s/valid? spec ::s/invalid)))
  (assert (= ::s/invalid (s/conform spec ::s/invalid))))

(doseq [[spec input output] [[::lift-first ::s/invalid :lifted]
                             [::keyword-or-lift ::s/invalid [:lift :lifted]]
                             [::optional-lifter {:lift-marker ::s/invalid} {:lift-marker :lifted}]]]
  (assert (s/valid? spec input))
  (assert (= output (s/conform spec input))))

(prn :folding-ranges-conformance-ok)

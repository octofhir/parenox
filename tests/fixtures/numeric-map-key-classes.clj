(ns find-keys (:require [clojure.spec.alpha :as s]))

(defn lookup {:t/sig '[:-> #{:hit}]} [] (get {1 :hit} 1N :missing))
(defn path {:t/sig '[:-> #{:hit}]} []
  (get-in {1 {:field :hit}} [1N :field] :missing))
(defn contains {:t/sig '[:-> #{true}]} [] (contains? {1 :hit} 1N))
(defn found {:t/sig '[:-> (s/tuple #{"x"})]} []
  (mapv first (val (find {1 [["x"]]} 1N))))
(defn wrapped {:t/sig '[(s/and int? #{1}) :-> #{:hit}]}
  [lookup] (get {1 :hit} lookup :missing))
(defn required {:t/sig '[:-> #{:hit}]} []
  (try (req! {1 :hit} 1N)
       (catch IllegalArgumentException _ :missing)))
(defn no-wrap {:t/sig '[:-> #{:missing}]} []
  (get {1 :hit} 4294967297N :missing))
(defn unknown-hash {:t/sig '[integer? :-> #{:hit :missing}]} [key]
  (case key
    1 (get {1 :hit 2 :a 3 :b 4 :c 5 :d 6 :e 7 :f 8 :g 9 :h} key :missing)
    :hit))
(def ^:private large-ratio
  10000000000000000000000000000000000000000/10000000000000000000000000000000000000000)
(defn ratio-key {:t/sig '[:-> #{:hit}]} [] (get {1 :hit} large-ratio :missing))
(defn ratio-index {:t/sig '[:-> #{:hit}]} [] (get [:zero :hit] large-ratio :missing))
(defn fixed-ratio? {:t/sig '[:-> #{false}]} [] (int? large-ratio))
(defn fractional-integer? {:t/sig '[:-> #{false}]} []
  (integer? 1/9999999999999999999999999999999999999999))
(defn projected {:t/sig '[:-> #{:hit}]} []
  (get (select-keys {1 :hit :extra false} [1]) 1N :missing))

(defn removed {:t/sig '[:-> (s/tuple #{:missing} #{:keep})]} []
  (let [remaining (dissoc {1 :old 2 :keep 3 :other} 1N 3)]
    [(get remaining 1 :missing)
     (val (find remaining 2N))]))

(defn nested-projection {:t/sig '[:-> #{:hit}]} []
  (get (:nested (select-keys {:nested {1 :hit} :discard true} [:nested]))
       1N :missing))

(defn nested-excess {:t/sig '[:-> #{:hit}]} []
  (let [{:keys [discard] :excess remaining} {:nested {1 :hit} :discard true}]
    (get (:nested remaining) 1N :missing)))

(defn nested-default {:t/sig '[:-> #{:hit}]} []
  (let [{:keys [nested] :or {nested {1 :hit}} :all settled} {}]
    (get (:nested settled) 1N :missing)))

(defn nested-default-overridden {:t/sig '[:-> #{:live}]} []
  (let [{:keys [nested] :or {nested {1 :backup}} :select settled} {:nested {1 :live}}]
    (get (:nested settled) 1N :missing)))

(defn nested-removal {:t/sig '[:-> #{:hit}]} []
  (get (:nested (dissoc {:nested {1 :hit} :discard true} :discard))
       1N :missing))

(defn replaced {:t/sig '[:-> (s/tuple #{:new} #{true})]} []
  (let [updated (assoc {1 :old} 1N :new)]
    [(get updated 1N :missing) (int? (key (find updated 1N)))]))

(defn sequential-write {:t/sig '[:-> #{:new}]} []
  (get (assoc {} 1 :seed 1N :new) 1N :missing))

(defn new-fixed {:t/sig '[:-> #{:new}]} []
  (get (assoc nil 1 :new) 1N :missing))

(defn alternative-contains {:t/sig '[boolean? :-> #{true}]} [which]
  (contains? {1 nil} (if which 1 1N)))

(defn alternative-found {:t/sig '[boolean? :-> #{:hit}]} [which]
  (val (find {1 :hit} (if which 1 1N))))

(defn alternative-required {:t/sig '[boolean? :-> #{:hit}]} [which]
  (try (req! {1 :hit} (if which 1 1N))
       (catch IllegalArgumentException _ :missing)))

(defn alternative-required-addresses {:t/sig '[boolean? :-> #{:one :two}]} [which]
  (try (req! {1 :one 2 :two} (if which 1N 2N))
       (catch IllegalArgumentException _ :missing)))

(defn alternative-removed {:t/sig '[boolean? :-> #{:missing}]} [which]
  (get (dissoc {1 :hit} (if which 1 1N)) 1 :missing))

(defn merged-top {:t/sig '[:-> #{:hit}]} []
  (get (merge {1 :hit} {:other :else}) 1N :missing))

(defn merged-child {:t/sig '[:-> #{:right}]} []
  (get (:nested (merge {:nested {1 :left}} {:nested {1 :right}})) 1N :missing))

(defn merged-with-nil {:t/sig '[:-> #{:hit}]} []
  (get (merge nil {1 :hit} nil) 1N :missing))

(defn merged-opaque {:t/sig '[{1 #{:hit}} :-> #{:missing :hit}]} [abstract]
  (get (:nested (merge {:nested {1 :hit}} {:nested abstract})) 1N :missing))

(defn assoc-unchanged {:t/sig '[:-> #{:hit}]} []
  (get (:nested (assoc {:nested {1 :hit}} :tag true)) 1N :missing))

(defn assoc-new-child {:t/sig '[:-> #{:hit}]} []
  (get (:nested (assoc {} :nested {1 :hit})) 1N :missing))

(defn assoc-opaque {:t/sig '[{1 #{:hit}} :-> #{:missing :hit}]} [abstract]
  (get (:nested (assoc {:nested {1 :hit}} :nested abstract)) 1N :missing))

(defn assoc-possible-write {:t/sig '[{1 #{:hit}} boolean? :-> #{:missing :hit}]} [abstract which]
  (let [k (if which :nested :tag)]
    (get (:nested (assoc {:nested {1 :hit}} k abstract)) 1N :missing)))

(defn zipped-single {:t/sig '[:-> #{:hit}]} []
  (get (zipmap [1] [:hit]) 1N :missing))

(defn zipped-duplicate {:t/sig '[:-> (s/tuple #{:new} #{true})]} []
  (let [out (zipmap [1 1N] [:old :new])]
    [(get out 1N :missing) (int? (key (find out 1)))]))

(defn path-update {:t/sig '[:-> #{:hit}]} []
  (get-in (update {:table {1 :hit} :on false} :on not)
          [:table 1N] :missing))

(defn path-insert {:t/sig '[:-> #{:hit}]} []
  (get-in (assoc-in nil [:outer :table] {1 :hit})
          [:outer :table 1N] :missing))

(defn path-return {:t/sig '[:-> #{:hit}]} []
  (get-in (update-in {:outer {}} [:outer :table] (fn [_] {1 :hit}))
          [:outer :table 1N] :missing))

(defn path-opaque {:t/sig '[{1 #{:hit}} :-> #{:missing :hit}]} [row]
  (get-in (assoc-in {:outer {:table {1 :hit}}} [:outer :table] row)
          [:outer :table 1N] :missing))

;; JVM-only assertions.
(assert (= [:hit :hit true ["x"] :hit :missing]
           [(lookup) (path) (contains) (found) (required) (no-wrap)]))
(assert (= :hit (wrapped (short 1))))
(assert (= java.lang.Long (class (key (find {1 :hit} 1N)))))
(let [by-class (fn [a b] (compare [(.getName (class a)) a]
                                 [(.getName (class b)) b]))
      sorted (sorted-map-by by-class 1 :hit)]
  (assert (= :missing (get sorted 1N :missing)))
  (assert (nil? (find sorted 1N)))
  (assert (= :missing (merged-opaque sorted)))
  (assert (= [:missing :missing :hit]
             [(assoc-opaque sorted) (assoc-possible-write sorted true)
              (assoc-possible-write sorted false)]))
  (assert (= :missing (path-opaque sorted))))
(let [key (proxy [java.math.BigInteger] ["1"] (hashCode [] 37))]
  (assert (integer? key))
  (assert (= :missing (unknown-hash key))))
(assert (= [clojure.lang.BigInt :hit :hit false false]
           [(class large-ratio) (ratio-key) (ratio-index) (fixed-ratio?) (fractional-integer?)]))
(assert (= :hit (projected)))
(assert (every? #(contains? (select-keys {1 nil} [%]) 1N) [1 1N]))
(assert (= [clojure.lang.BigInt java.lang.Long]
           (mapv (fn [keys] (class (key (find (select-keys {1 :hit} keys) 1))))
                 [[1N 1] [1 1N]])))
(assert (= [:missing :keep] (removed)))
(assert (= [:hit :hit :hit :live :hit]
           [(nested-projection) (nested-excess) (nested-default)
            (nested-default-overridden) (nested-removal)]))
(assert (= [[:new true] :new :new]
           [(replaced) (sequential-write) (new-fixed)]))
(assert (= [[true :hit :hit :one :missing] [true :hit :hit :two :missing]]
           (mapv (fn [which]
                   [(alternative-contains which) (alternative-found which)
                    (alternative-required which) (alternative-required-addresses which)
                    (alternative-removed which)])
                 [true false])))
(assert (= [:hit :right :hit] [(merged-top) (merged-child) (merged-with-nil)]))
(assert (= [:hit :hit] [(assoc-unchanged) (assoc-new-child)]))
(assert (= [:hit [:new true]] [(zipped-single) (zipped-duplicate)]))
(assert (= [:new clojure.lang.BigInt]
           (let [out (zipmap [1N 1] [:old :new])]
             [(get out 1) (class (key (find out 1)))])))
(assert (= [:hit :hit :hit] [(path-update) (path-insert) (path-return)]))
(println "Clojure/JVM numeric map keys: 23 assertions passed")

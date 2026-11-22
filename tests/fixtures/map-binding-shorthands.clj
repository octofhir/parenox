(ns demo.shorthand
  (:require [clojure.spec.alpha :as s]
            [clojure.string :as str]
            [demo.catalog :as-alias catalog]))

(defn fields {:t/sig '[:-> (s/tuple number? string? nil?)]} []
  (let [{:syms [item] :catalog/syms [label] :syms! [present & 'verified]}
        '{item 7 :item "keyword" catalog/label "text" present nil verified false}]
    [item label present]))

(defn default-name {:t/sig '[:-> #{"Unknown"}]} []
  (let [{:syms [catalog/name] :or {'catalog/name "Unknown"}} {}] name))

(defn namespace-default {:t/sig '[:-> string?]} []
  (let [{:keys [::default] :as m} {::default "fallback"}]
    (m ::default default)))

(defn alias-default {:t/sig '[:-> number?]} []
  (let [{:keys [::catalog/value]} {::catalog/value 8}] value))

(defn slash {:t/sig '[:-> number?]} []
  (let [{:keys [:/]} {:/ 9}] /))

(defn projected-default {:t/sig '[:-> (s/tuple #{"guest"} #{"guest"})]} []
  (let [{:keys [name] :or {name "guest"} :select selected :all merged} nil]
    [(get selected :name) (get merged :name)]))

(defn projected-nil {:t/sig '[:-> (s/tuple nil? number? nil?)]} []
  (let [{:keys [name] :or {name "guest"} :select selected :all merged}
        {:name nil :extra 8}]
    [(get selected :name) (get merged :extra) (get selected :extra)]))

(defn projected-key-default {:t/sig '[:-> (s/tuple number? #{false})]} []
  (let [{:keys [name & :priority] :or {name "guest" :priority 5}
         :select selected :all merged} {:other false}]
    [(get selected :priority) (get merged :other)]))

(defn missing-keys {:t/sig '[:-> (s/tuple nil? nil? #{true} #{true} #{false})]} []
  (let [{:keys! [item & :verified] :req! {n :name} :missing absent}
        {:name nil}]
    [item n (contains? absent :item) (contains? absent :verified)
     (contains? absent :name)]))

(defn missing-none {:t/sig '[:-> nil?]} []
  (let [{:syms! [present & 'verified] :missing absent}
        '{present nil verified false}]
    absent))

(defn nested-missing {:t/sig '[:-> (s/tuple nil? #{true} #{true})]} []
  (let [{{:keys! [name] :missing local} :inside :missing absent} {}]
    [name (contains? local :name) (contains? (:inside absent) :name)]))

(defn missing-parent {:t/sig '[:-> (s/tuple #{true} nil?)]} []
  (let [{:req! {{:keys [name]} :inside} :missing absent} {}]
    [(contains? absent :inside) (:inside absent)]))

(defn missing-child-wins {:t/sig '[:-> (s/tuple #{true} #{true})]} []
  (let [{:req! {{:keys! [name]} :inside} :missing absent} {}]
    [(contains? absent :inside) (contains? (:inside absent) :name)]))

(defn missing-child-satisfied {:t/sig '[:-> nil?]} []
  (let [{{:keys! [name]} :inside :missing absent} {:inside {:name nil}}]
    absent))

(defn missing-child-rebound {:t/sig '[:-> #{false}]} []
  (let [{{:keys! [x] :missing collected} :inside
         collected :replacement
         :missing absent}
        {:replacement {:override true}}]
    (contains? (:inside absent) :x)))

(defn nested-excess {:t/sig '[:-> (s/tuple number? number? #{false})]} []
  (let [{{:keys [used]} :inside :excess leftover}
        {:inside {:used 7 :extra 9} :outside 11}]
    [(:extra (:inside leftover)) (:outside leftover)
     (contains? (:inside leftover) :used)]))

(defn empty-nested-excess {:t/sig '[:-> nil?]} []
  (let [{{:keys [used]} :inside :excess leftover} {:inside {:used 7}}]
    leftover))

(defn vector-excess {:t/sig '[:-> (s/tuple number? number?)]} []
  (let [{[used] :inside :excess leftover} {:inside [7 8] :outside 11}]
    [used (:outside leftover)]))

(defn rebound-excess {:t/sig '[:-> #{true}]} []
  (let [{{:keys [used] :excess collected} :inside
         collected :replacement
         :excess leftover}
        {:inside {:used 7 :extra false} :replacement {:updated true}}]
    (contains? (:inside leftover) :updated)))

(defn excess-and-missing {:t/sig '[:-> (s/tuple nil? number? #{true})]} []
  (let [{{:keys! [item]} :inside :excess leftover :missing absent}
        {:inside {:extra 7}}]
    [item (:extra (:inside leftover)) (contains? (:inside absent) :item)]))

(defn nested-selection {:t/sig '[:-> (s/tuple number? nil? number? number?)]} []
  (let [{{:keys [take]} :inside :select picked :all complete}
        {:inside {:take 7 :other 9} :outside 11}]
    [(get-in picked [:inside :take]) (get-in picked [:inside :other])
     (get-in complete [:inside :other]) (:outside complete)]))

(defn nested-defaults {:t/sig '[:-> (s/tuple #{"guest"} #{"guest"} number?)]} []
  (let [{{:keys [name] :or {name "guest"}} :inside :select picked :all complete}
        {:outside 11}]
    [(get-in picked [:inside :name]) (get-in complete [:inside :name])
     (:outside complete)]))

(defn nested-nil-selection {:t/sig '[:-> (s/tuple #{true} nil?)]} []
  (let [{{:keys [name]} :inside :select picked} {:inside nil}]
    [(contains? picked :inside) (:inside picked)]))

(defn rebound-selection {:t/sig '[:-> #{true}]} []
  (let [{{:keys [name] :select picked} :inside
         picked :replacement
         :select kept}
        {:inside {:name "before"} :replacement {:after true}}]
    (contains? (:inside kept) :after)))

(defn projection-order {:t/sig '[:-> number?]} []
  (let [{{:keys [value] :all shared} :inside :select shared :all complete}
        {:inside {:value 7 :extra 9} :outside 11}]
    (get-in complete [:inside :inside :value])))

(defn alternating {:t/sig '[:-> (s/tuple #{:item} number? #{false} #{true})]} []
  (let [args (list :item 7 :flag false)
        {:keys [item flag] :as opts} args]
    [(first args) item flag (map? opts)]))

(defn trailing {:t/sig '[:-> (s/tuple #{:last} #{:hit} #{:other})]} []
  (let [{:keys [item nested] :select picked :all opts}
        (list :item :first :item :previous {:item :last :nested {1 :hit} :other :other})]
    [item (get (:nested picked) 1N) (:other opts)]))

(defn single {:t/sig '[:-> (s/tuple #{:hit} #{:as-is} nil? #{true})]} []
  (let [{:keys [item]} '({:item {1 :hit}})
        {:as token} (list :as-is)
        {:as absent} (list nil)
        {:as empty-opts} (list)]
    [(get item 1N) token absent (map? empty-opts)]))

(defn sequence-inputs {:t/sig '[:-> (s/tuple #{:tail} #{:last} #{:hit} #{:hit})]} []
  (let [{:keys [value]} (seq [:value :tail])
        {value-from-nil :value} (list :value :last nil)
        {num 1} (list 1 :before 1N :hit)
        {:keys [nested]} (list :nested {1 :hit})]
    [value value-from-nil num (get nested 1N)]))

(defn choose {:t/sig '[boolean? :-> string?]} [prefer-map?]
  (let [args (if prefer-map? {:path " x "} (list :path " y "))
        {:keys [path] :as saved} args]
    (str (str/trim path) (str/trim (:path saved)))))

(defn names {:t/sig '[boolean? :-> string?]} [short?]
  (let [options (if short? (list :name " x ") (list :unused false :name " y "))
        {:keys [name] :select chosen} options]
    (str (str/trim name) (str/trim (:name chosen)))))

(defn optional {:t/sig '[boolean? :-> string?]} [enabled?]
  (let [options (if enabled? (list :path " x ") nil)
        {:keys [path] :or {path "fallback"}} options]
    (str/trim path)))

(defn vector-entry {:t/sig '[:-> #{:hit}]} []
  (let [{:keys [nested]} (list :other false [:nested {1 :hit}])]
    (get nested 1N)))

(defn map-entry {:t/sig '[:-> #{:hit}]} []
  (let [{:keys [nested]} (list :other false (first {:nested {1 :hit}}))]
    (get nested 1N)))

(defn numeric-entry {:t/sig '[:-> #{true}]} []
  (let [{:as options} (list 1 :before [1N :after])]
    (and (= :after (get options 1N)) (int? (first (keys options))))))

(defn entry-sequence-tail {:t/sig '[:-> (s/tuple #{:last} #{:hit} #{false})]} []
  (let [{:keys [state nested flag]}
        (list :state :first
              (list (first {:nested {1 :hit}})
                    (first {:state :last})
                    (first {:flag false})))]
    [state (get nested 1N) flag]))

(defn empty-entry-sequence-tail {:t/sig '[:-> #{:retained}]} []
  (let [{:keys [item]} (list :item :retained (list))]
    item))

;; Diagnostic cases.
(defn wrong-operand {:t/sig '[:-> number?]} []
  (let [{:keys [::default] :as m} {::default "fallback"}] (inc default)))

(defn required-symbol {:t/sig '[:-> number?]} []
  (let [{:cfg/syms! [label & 'cfg/required]} '{cfg/label 7}] label))

(defn required-shorthand-order {:t/sig '[:-> number?]} []
  (let [{:keys! [early late & :also-missing]} {}]
    (+ early late)))

(defn required-map-order {:t/sig '[:-> number?]} []
  (let [{:req! {x :first y :second} :keys! [z]} {}]
    (+ x y z)))

;; JVM-only assertions.
(assert (= [7 "text" nil] (fields)))
(assert (= "Unknown" (default-name)))
(assert (= "fallback" (namespace-default)))
(assert (= 8 (alias-default)))
(assert (= 9 (slash)))
(assert (= ["guest" "guest"] (projected-default)))
(assert (= [nil 8 nil] (projected-nil)))
(assert (= [5 false] (projected-key-default)))
(assert (= [nil nil true true false] (missing-keys)))
(assert (nil? (missing-none)))
(assert (= [[nil true true] [true nil] [true true] nil]
           [(nested-missing) (missing-parent)
            (missing-child-wins) (missing-child-satisfied)]))
(assert (try (wrong-operand) false (catch ClassCastException _ true)))
(assert (try (required-symbol)
             false
             (catch IllegalArgumentException e
               (= "Missing required key: cfg/required" (.getMessage e)))))
(assert (try (required-shorthand-order) false
             (catch IllegalArgumentException e
               (= "Missing required key: :early" (.getMessage e)))))
(assert (try (required-map-order) false
             (catch IllegalArgumentException e
               (= "Missing required key: :second" (.getMessage e)))))
(assert (false? (missing-child-rebound)))
(assert (= [[9 11 false] nil [7 11] true [nil 7 true]]
           [(nested-excess) (empty-nested-excess) (vector-excess)
            (rebound-excess) (excess-and-missing)]))
(assert (= [[7 nil 9 11] ["guest" "guest" 11] [true nil] true 7]
           [(nested-selection) (nested-defaults) (nested-nil-selection)
            (rebound-selection) (projection-order)]))
(assert (= [[:item 7 false true] [:last :hit :other]
            [:hit :as-is nil true] [:tail :last :hit :hit]]
           [(alternating) (trailing) (single) (sequence-inputs)]))
(assert (= [["yy" "yy" "fallback"] ["xx" "xx" "x"]]
           (mapv (juxt choose names optional) [false true])))
(assert (= [:hit :hit true] [(vector-entry) (map-entry) (numeric-entry)]))
(assert (= [[:last :hit false] :retained]
           [(entry-sequence-tail) (empty-entry-sequence-tail)]))
(println "Clojure/JVM map binding shorthands: 22 assertions passed")

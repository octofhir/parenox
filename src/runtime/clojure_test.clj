(ns parenox.runtime.clojure-test
  (:require [clojure.java.io :as io]
            [clojure.string :as str]
            [clojure.test :as t])
  (:import (clojure.lang Compiler$CompilerException)))

(defn- local-file [source]
  (let [file (io/file source)]
    (if (.isAbsolute file)
      (when (.isFile file) file)
      (if-let [resource (io/resource source)]
        (when (= "file" (.getProtocol resource))
          (io/file resource))
        (when (.isFile file) file)))))

(defn- position [root resolved line]
  (let [path (.toPath (.getCanonicalFile resolved))]
    [(if (.startsWith path root) (.relativize root path) path) line]))

(defn- same-file-line? [^StackTraceElement frame file line]
  (and (= line (.getLineNumber frame))
       (= file (.getFileName frame))))

(defn- test-location [root {:keys [type file line actual]} {:keys [ns] source :file}]
  (when (and ns source file (pos-int? line)
             (or (= file source) (= file (.getName (io/file source)))))
    (let [prefix (str (munge (str (ns-name ns))) "$")
          frames (if (and (= :error type) (instance? Throwable actual))
                   (.getStackTrace ^Throwable actual)
                   (.getStackTrace (Thread/currentThread)))]
      (when (some (fn [^StackTraceElement frame]
                    (and (same-file-line? frame file line)
                         (str/starts-with? (.getClassName frame) prefix)))
                  frames)
        (when-let [resolved (local-file source)]
          (position root resolved line))))))

(defn- frame-namespace [known ^String class-name]
  (let [[^String base generated] (str/split class-name #"[$/]" 2)
        dot (.lastIndexOf base ".")]
    (cond
      (and generated (contains? known base)) base
      (pos? dot) (known (subs base 0 dot)))))

(defn- stack-location [root frames relevant?]
  (let [namespace-classes (into #{} (map (comp munge str ns-name)) (all-ns))]
    (some (fn [^StackTraceElement frame]
            (let [file (.getFileName frame)
                  line (.getLineNumber frame)]
              (when (and (pos-int? line) file (relevant? frame)
                         (or (str/ends-with? file ".clj") (str/ends-with? file ".cljc")))
                (when-let [^String namespace (frame-namespace namespace-classes (.getClassName frame))]
                  (let [package (subs namespace 0 (inc (.lastIndexOf namespace ".")))
                        resource (str (str/replace package "." "/") file)]
                    (when-let [resolved (local-file resource)]
                      (position root resolved line)))))))
          frames)))

(defn- error-location [root ^Throwable error]
  (stack-location root (.getStackTrace error) (constantly true)))

(defn- assertion-location [root {:keys [file line]}]
  (when (and file (pos-int? line))
    (stack-location root (.getStackTrace (Thread/currentThread))
                    #(same-file-line? % file line))))

(defn- reporter [report findings]
  (let [root (.toPath (.getCanonicalFile (io/file ".")))]
    (fn [{:keys [type actual message] :as event}]
      (when (#{:fail :error} type)
        (let [{:keys [ns name] :as metadata} (meta (first t/*testing-vars*))
              [file line] (or (when (and (= type :error) (instance? Throwable actual))
                                (error-location root actual))
                              (test-location root event metadata)
                              (when (= type :fail) (assertion-location root event)))
              test (when ns (str (ns-name ns) "/" name))]
          (swap! findings conj [type test (some-> file str) line (when (string? message) message)])
          (when file
            (t/with-test-out
              (println (str file ":" line ": " (clojure.core/name type)
                            (when test (str " in " test))))))))
      (report event))))

(defn- test-var [name]
  (let [v (find-var name)]
    (when-not (:test (meta v))
      (throw (ex-info (str "Expected a clojure.test var: " name) {:test name})))
    v))

(defn- run-vars [namespaces vars]
  (let [by-ns (group-by (comp ns-name :ns meta) vars)]
    (binding [t/*report-counters* (ref t/*initial-report-counters*)]
      (doseq [name namespaces
              :let [ns (the-ns name)]]
        (t/do-report {:type :begin-test-ns :ns ns})
        (t/test-vars (get by-ns name))
        (t/do-report {:type :end-test-ns :ns ns}))
      (let [summary (assoc @t/*report-counters* :type :summary)]
        (t/do-report summary)
        summary))))

(defn- compiler-location [root cause]
  (let [{:clojure.error/keys [source line column]} (ex-data cause)
        [source line] (if (and source line)
                        [source line]
                        (when (instance? Compiler$CompilerException cause)
                          [(.-source ^Compiler$CompilerException cause)
                           (.-line ^Compiler$CompilerException cause)]))]
    (when (and (string? source) (pos-int? line))
      (when-let [file (local-file source)]
        (conj (position root file line) (when (pos-int? column) column))))))

(defn- incomplete [phase ^Throwable error]
  (let [root (.toPath (.getCanonicalFile (io/file ".")))
        causes (take 64 (take-while some? (iterate #(.getCause ^Throwable %) error)))
        [file line column] (or (some #(compiler-location root %) causes)
                               (when (= phase :execution) (error-location root error)))
        message (or (.getMessage ^Throwable (last causes))
                    (.getMessage error)
                    (.getName (class error)))]
    (.printStackTrace error)
    [:run-error phase message (some-> file str) line column]))

(let [[result mode & names] *command-line-args*
      names (distinct (map symbol names))
      vars? (= mode "vars")
      namespaces (if vars? (distinct (map (comp symbol namespace) names)) names)
      phase (volatile! :load)
      summary (try
                (apply require namespaces)
                (vreset! phase :selection)
                (let [vars (when vars? (mapv test-var names))
                      findings (atom [])]
                  (vreset! phase :execution)
                  (let [{:keys [test pass fail error]}
                        (binding [t/report (reporter t/report findings)]
                          (if vars?
                            (run-vars namespaces vars)
                            (apply t/run-tests namespaces)))]
                    [test (+ pass fail error) fail error @findings]))
                (catch Throwable error (incomplete @phase error)))]
  (binding [*print-length* nil *print-level* nil *print-readably* true]
    (spit result (pr-str summary)))
  (shutdown-agents))

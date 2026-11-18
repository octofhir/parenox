(ns parenox.runtime.clojure-test
  (:require [clojure.java.io :as io]
            [clojure.string :as str]
            [clojure.test :as t]))

(defn- local-file [source]
  (let [file (io/file source)]
    (if (and (.isAbsolute file) (.isFile file))
      file
      (when-let [resource (io/resource source)]
        (when (= "file" (.getProtocol resource))
          (io/file resource))))))

(defn- position [root resolved line]
  (let [path (.toPath (.getCanonicalFile resolved))]
    [(if (.startsWith path root) (.relativize root path) path) line]))

(defn- test-location [root {:keys [type file line actual]} {:keys [ns] source :file}]
  (when (and ns source file (pos-int? line)
             (or (= file source) (= file (.getName (io/file source)))))
    (let [prefix (str (munge (str (ns-name ns))) "$")
          frames (if (and (= :error type) (instance? Throwable actual))
                   (.getStackTrace ^Throwable actual)
                   (.getStackTrace (Thread/currentThread)))]
      (when (some (fn [^StackTraceElement frame]
                    (and (= line (.getLineNumber frame))
                         (= file (.getFileName frame))
                         (str/starts-with? (.getClassName frame) prefix)))
                  frames)
        (when-let [resolved (local-file source)]
          (position root resolved line))))))

(defn- error-location [root ^Throwable error]
  (let [namespace-classes (into #{} (map (comp munge str ns-name)) (all-ns))]
    (some (fn [^StackTraceElement frame]
            (let [file (.getFileName frame)
                  line (.getLineNumber frame)
                  [prefix generated] (str/split (.getClassName frame) #"[$/]" 2)]
              (when (and generated (contains? namespace-classes prefix)
                         (pos-int? line) file
                         (or (str/ends-with? file ".clj") (str/ends-with? file ".cljc")))
                (let [package (subs prefix 0 (inc (.lastIndexOf ^String prefix ".")))
                      resource (str (str/replace package "." "/") file)]
                  (when-let [resolved (local-file resource)]
                    (position root resolved line))))))
          (.getStackTrace error))))

(defn- reporter [report findings]
  (let [root (.toPath (.getCanonicalFile (io/file ".")))]
    (fn [{:keys [type actual message] :as event}]
      (when (#{:fail :error} type)
        (let [{:keys [ns name] :as metadata} (meta (first t/*testing-vars*))
              [file line] (or (when (and (= type :error) (instance? Throwable actual))
                                (error-location root actual))
                              (test-location root event metadata))
              test (when ns (str (ns-name ns) "/" name))]
          (swap! findings conj [type test (some-> file str) line (when (string? message) message)])
          (when file
            (t/with-test-out
              (println (str file ":" line ": " (clojure.core/name type)
                            (when test (str " in " test))))))))
      (report event))))

(let [[result & names] *command-line-args*
      namespaces (distinct (map symbol names))]
  (apply require namespaces)
  (let [findings (atom [])
        {:keys [test pass fail error]} (binding [t/report (reporter t/report findings)]
                                        (apply t/run-tests namespaces))]
    (binding [*print-length* nil *print-level* nil *print-readably* true]
      (spit result (pr-str [test (+ pass fail error) fail error @findings])))
    (shutdown-agents)))

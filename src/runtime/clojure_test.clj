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

(defn- test-location [{:keys [type file line actual]} {:keys [ns] source :file}]
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
          (let [root (.toPath (.getCanonicalFile (io/file ".")))
                path (.toPath (.getCanonicalFile resolved))]
            [(if (.startsWith path root) (.relativize root path) path) line]))))))

(defn- reporter [report]
  (fn [{:keys [type] :as event}]
    (when (#{:fail :error} type)
      (let [{:keys [ns name] :as metadata} (meta (first t/*testing-vars*))]
        (when-let [[file line] (test-location event metadata)]
          (t/with-test-out
            (println (str file ":" line ": " (clojure.core/name type) " in " (ns-name ns) "/" name))))))
    (report event)))

(let [[result & names] *command-line-args*
      namespaces (distinct (map symbol names))]
  (apply require namespaces)
  (let [summary (binding [t/report (reporter t/report)]
                  (apply t/run-tests namespaces))]
    (spit result (cond
                   (zero? (:test summary)) "empty"
                   (t/successful? summary) "pass"
                   :else "fail"))
    (shutdown-agents)))

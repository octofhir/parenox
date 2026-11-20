(ns portable.macros)

(defn decorate [value]
  [:definition value])

(defmacro decorate-once [form]
  `(let [value# ~form]
     (decorate value#)))

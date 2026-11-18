use std::fmt::Write;

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Shape {
    Chain,
    Shared,
}

pub fn library(shape: Shape, levels: usize) -> String {
    let mut source = String::from(
        "(ns expansion (:require [clojure.spec.alpha :as s]))\n(def status :ready)\n(s/def ::level0 string?)\n",
    );
    for level in 1..=levels {
        let previous = level - 1;
        let spec = match shape {
            Shape::Chain => format!("(s/keys :req-un [::level{previous}])"),
            // Resolved function types share their signatures by Arc.
            Shape::Shared => format!(
                "(s/fspec :args (s/cat :left ::level{previous} :right ::level{previous}) :ret nil?)"
            ),
        };
        writeln!(source, "(s/def ::level{level} {spec})").unwrap();
    }
    writeln!(
        source,
        "(defn submit {{:t/sig '[::level{levels} :-> nil?]}} [_] nil)"
    )
    .unwrap();
    source
}

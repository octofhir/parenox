//! Standard signature help for source arglists and checked calls.
use std::borrow::Cow;

use parenox_analysis::CallSignatures;
use serde_json::{Value, json};

pub fn render(info: &CallSignatures, origin: Option<&str>) -> Value {
    let documentation = match (info.documentation.as_deref(), origin) {
        (Some(docs), Some(origin)) => Some(Cow::Owned(format!("{docs}\n\n{origin}"))),
        (docs, origin) => docs.or(origin).map(Cow::Borrowed),
    };
    let signatures: Vec<_> = info
        .signatures
        .iter()
        .map(|signature| {
            let parameters: Vec<_> = signature
                .parameters
                .iter()
                .map(|label| json!({"label":label}))
                .collect();
            let mut label = format!("({} [{}])", info.name, signature.parameters.join(" "));
            if let Some(result) = &signature.result {
                label.push_str(" -> ");
                label.push_str(result);
            }
            let mut item = json!({"label":label,"parameters":parameters});
            if let Some(documentation) = &documentation {
                item["documentation"] = json!(documentation);
            }
            item
        })
        .collect();
    let mut help = json!({"signatures":signatures,"activeSignature":info.active_signature});
    if let Some(active) = info.active_parameter {
        help["activeParameter"] = json!(active);
    }
    help
}

use super::*;

#[test]
fn source_contract_links_follow_the_provider_overlay_and_its_uri() {
    let fixture = Fixture::new("source contract locations");
    let provider_file = fixture.0.join("src/provider.clj");
    let caller_file = fixture.0.join("src/app.clj");
    let provider = uri(&provider_file);
    let editor_uri = format!(
        "file://localhost{}",
        provider.strip_prefix("file://").unwrap()
    );
    let caller = uri(&caller_file);
    let declaration = r#"(ns provider (:require [clojure.spec.alpha :as s]))
(defn cast {:t/sig '[string? :-> string?]} [text] text)
(defn promote [text] text)
(s/fdef promote :args (s/cat :text string?) :ret string?)
"#;
    let source = r#"(ns app (:require [provider :as p]))
(defn cast-bad {:t/sig '[:-> string?]} [] (p/cast 1))
(defn promote-bad {:t/sig '[:-> string?]} [] (p/promote 2))
"#;
    let shifted = format!(
        "; new heading\n{}",
        declaration.replace("(s/fdef", "#_\"😀\" (s/fdef")
    );
    fs::write(&provider_file, declaration).unwrap();
    fs::write(&caller_file, source).unwrap();
    let open = |uri: &str, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":uri,"languageId":"clojure","version":1,"text":text}
        }})
    };
    let change = |version, text: &str| {
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":editor_uri,"version":version},"contentChanges":[{"text":text}]
        }})
    };
    let replies = transcript(&[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "rootUri":uri(&fixture.0),"capabilities":{"textDocument":{"publishDiagnostics":{"relatedInformation":true}}}
        }}),
        open(&caller, source),
        open(&editor_uri, declaration),
        change(2, &shifted),
        change(3, declaration),
        json!({"jsonrpc":"2.0","id":2,"method":"shutdown"}),
        json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    let publications: Vec<_> = replies
        .iter()
        .filter_map(|message| {
            let params = &message["params"];
            (message["method"] == "textDocument/publishDiagnostics" && params["uri"] == caller)
                .then_some(params)
        })
        .collect();
    assert_eq!(publications.len(), 4, "{replies:?}");
    let mut primary = Vec::new();
    for (params, (uri, text)) in publications.into_iter().zip([
        (&provider, declaration),
        (&editor_uri, declaration),
        (&editor_uri, shifted.as_str()),
        (&editor_uri, declaration),
    ]) {
        assert_eq!(params["version"], 1);
        let findings: Vec<_> = params["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|finding| finding["code"] == "PNX2322")
            .collect();
        assert_eq!(findings.len(), 2, "{params}");
        for (finding, (declaring_form, anchor)) in findings.into_iter().zip([
            ("{:t/sig", "'[string? :-> string?]"),
            ("(s/fdef", "promote"),
        ]) {
            let (line, row) = text
                .lines()
                .enumerate()
                .find(|(_, row)| row.contains(declaring_form))
                .unwrap();
            let start = row[..row.find(anchor).unwrap()].encode_utf16().count();
            let related = finding["relatedInformation"].as_array().unwrap();
            assert_eq!(related.len(), 1);
            assert_eq!(
                related[0]["location"],
                json!({"uri":uri,"range":{
                    "start":{"line":line,"character":start},
                    "end":{"line":line,"character":start + anchor.encode_utf16().count()}
                }})
            );
            let mut primary_finding = finding.clone();
            primary_finding
                .as_object_mut()
                .unwrap()
                .remove("relatedInformation");
            primary.push(primary_finding);
        }
    }
    let (primary, remainder) = primary.as_chunks::<2>();
    assert!(remainder.is_empty());
    for pair in &primary[1..] {
        assert_eq!(pair, &primary[0]);
    }
    assert_eq!(fs::read_to_string(provider_file).unwrap(), declaration);
}

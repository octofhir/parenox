//! Render checked types, arglists and Clojure docstrings for editor hover.
use parenox_analysis::{Session, SpecContractProposal};
use serde_json::{Value, json};

use crate::{clojuredocs::ClojureDocs, completion::DependencyCompletions, text::Range};

pub fn spec(proposal: &SpecContractProposal, markdown: bool, range: Option<Range>) -> Value {
    let mut checked = Vec::new();
    if let Some(input) = &proposal.input {
        checked.push(format!("Input type {input}"));
    }
    if let Some(result) = &proposal.result {
        checked.push(format!("Result type {result}"));
    }
    checked.extend(
        proposal
            .unknown
            .iter()
            .map(|part| format!("{part} is unknown")),
    );
    checked.push(format!(
        "Inferred from s/def in {} at line {}",
        proposal.path, proposal.span.start.line
    ));
    render(&proposal.name, &[], &checked, None, markdown, range, None)
}

pub fn dependency(
    session: &Session,
    path: &str,
    offset: usize,
    dependencies: &DependencyCompletions,
    markdown: bool,
    range: Option<Range>,
    clojuredocs: &ClojureDocs,
) -> Option<Value> {
    let (namespace, name) = session.hover_archive_var(path, offset)?;
    let qualified = format!("{namespace}/{name}");
    let page = clojuredocs.find(&qualified);
    let info = dependencies.documentation(namespace, name);
    if page.is_none() && info.is_none() {
        return None;
    }
    Some(render(
        &qualified,
        info.as_ref().map_or(&[], |info| info.arglists.as_slice()),
        &[],
        info.as_ref().and_then(|info| info.documentation.as_deref()),
        markdown,
        range,
        page,
    ))
}

pub fn render(
    name: &str,
    arglists: &[String],
    checked: &[String],
    documentation: Option<&str>,
    markdown: bool,
    range: Option<Range>,
    page: Option<&Value>,
) -> Value {
    let arglists: Vec<_> = if arglists.is_empty() {
        texts(page, "arglists").collect()
    } else {
        arglists.iter().map(String::as_str).collect()
    };
    let mut sections = Vec::new();
    if markdown {
        let code = if arglists.is_empty() {
            name.to_owned()
        } else {
            format!("({name}\n {})", arglists.join("\n "))
        };
        sections.push(format!("```clojure\n{code}\n```"));
        if !checked.is_empty() {
            sections.push(format!("```text\n{}\n```", checked.join("\n")));
        }
    } else {
        sections.push(
            std::iter::once(name)
                .chain(checked.iter().map(String::as_str))
                .chain(arglists)
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    if let Some(documentation) = documentation
        .filter(|text| !text.trim().is_empty())
        .or_else(|| page?.get("doc")?.as_str())
    {
        sections.push(documentation.to_owned());
    }
    if let Some(page) = page {
        append_clojuredocs(&mut sections, page, markdown);
    }
    let kind = if markdown { "markdown" } else { "plaintext" };
    let mut hover = json!({"contents":{"kind":kind,"value":sections.join("\n\n")}});
    if let Some(range) = range {
        hover["range"] = json!(range);
    }
    hover
}

fn texts<'a>(page: Option<&'a Value>, field: &str) -> impl Iterator<Item = &'a str> {
    page.and_then(|page| page.get(field))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

fn append_clojuredocs(sections: &mut Vec<String>, page: &Value, markdown: bool) {
    for (field, title) in [
        ("examples", "Examples"),
        ("seeAlsos", "See also"),
        ("notes", "Notes"),
    ] {
        let mut items = texts(Some(page), field).peekable();
        if items.peek().is_none() {
            continue;
        }
        sections.push(if markdown {
            format!("**{title}**")
        } else {
            title.to_owned()
        });
        sections.extend(items.map(|item| {
            if markdown && field == "examples" {
                example_block(item)
            } else {
                item.to_owned()
            }
        }));
    }
}

fn example_block(source: &str) -> String {
    let longest = source
        .split(|ch| ch != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(3.max(longest + 1));
    format!("{fence}clojure\n{source}\n{fence}")
}

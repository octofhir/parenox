//! Derive reviewable named spec contracts from the current project snapshot.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use super::{FileCache, FileKind, ProjectReport, Session, SnapshotMode, Source};
use crate::checker::{Globals, Model, SnapshotWriter, SpecConversion};
use crate::reader::Span;
use crate::types::Type;

#[derive(Debug)]
pub struct SpecContractProposal {
    pub name: String,
    pub path: String,
    pub span: Span,
    pub form: String,
    pub input: Option<String>,
    pub result: Option<String>,
    pub unknown: Vec<&'static str>,
}

#[derive(Debug)]
pub struct ContractProposals {
    pub report: ProjectReport,
    pub edn: Option<String>,
    pub externs: Option<String>,
}

impl Session {
    /// Describe one named spec from the current checked source snapshot.
    pub fn spec_contract(&self, path: &str, name: &str) -> Option<SpecContractProposal> {
        self.describe_spec(path, name, !self.has_source_namespace("clojure.spec.alpha"))
    }

    fn describe_spec(
        &self,
        path: &str,
        name: &str,
        standard_spec: bool,
    ) -> Option<SpecContractProposal> {
        let file = self
            .files
            .get(path)
            .filter(|file| file.snapshot == self.snapshot)?;
        let declarations = file.declarations.as_ref()?;
        let key = declarations.spec_site(name)?.ok()?;
        let index = file
            .forms
            .partition_point(|form| form.span.end.offset <= key.start.offset);
        let span = file.forms.get(index)?.span;
        let conversion = standard_spec
            .then(|| self.specs.conversions.get(name))
            .flatten();
        let input = if matches!(
            conversion,
            Some(SpecConversion::WrapField(_) | SpecConversion::MapOrWrapField(_))
        ) {
            Some("Input (Any)".into())
        } else if standard_spec {
            self.specs.types.get(name).and_then(display_type)
        } else {
            None
        };
        let result = conversion.and_then(SpecConversion::proposal_result);
        let mut unknown = Vec::new();
        if self.specs.unchecked_input(name) || input.is_none() {
            unknown.push("Input domain");
        } else if result.is_none() {
            unknown.push("Full validation behavior of the input projection");
        }
        if result.is_none() {
            unknown.push("Conformed result");
        } else if !declarations.has_namespace_declaration()
            || self.specs.standalone_namespace(name, path).is_none()
        {
            unknown.push("Standalone registration");
        }
        if declarations.resolve_spec_keyword(name).as_deref() != Some(name) {
            unknown.push("Qualified registration name");
        }
        if !standard_spec {
            unknown.push("Behavior of the project's clojure.spec.alpha");
        }
        Some(SpecContractProposal {
            name: name.into(),
            path: path.into(),
            span,
            form: file.source.get(span.start.offset..span.end.offset)?.into(),
            input,
            result,
            unknown,
        })
    }

    /// Generate proposals and a standalone extern map without editing the source
    /// or any supplied externs. The extern map includes only supported conversions.
    pub fn propose_contracts(
        &mut self,
        sources: &[Source<'_>],
        externs: &[Source<'_>],
        requested: &[String],
    ) -> Result<ContractProposals, String> {
        let report = self.check_snapshot(sources, externs, &[], SnapshotMode::ExternSurface);
        if report
            .files
            .iter()
            .any(|file| file.diagnostics.iter().any(|d| d.severity == "error"))
        {
            return Ok(ContractProposals {
                report,
                edn: None,
                externs: None,
            });
        }
        let requested: BTreeSet<_> = requested.iter().map(String::as_str).collect();
        let standard_spec = !self.has_source_namespace("clojure.spec.alpha");
        let mut proposals = BTreeMap::new();
        for source in sources {
            let Some(declarations) = &self.files[source.path].declarations else {
                continue;
            };
            for name in declarations.spec_names() {
                if (requested.is_empty() || requested.contains(name))
                    && let Some(proposal) = self.describe_spec(source.path, name, standard_spec)
                {
                    proposals.insert(name, proposal);
                }
            }
        }
        if let Some(missing) = requested.iter().find(|name| !proposals.contains_key(*name)) {
            return Err(format!("No source declaration for spec {missing}"));
        }

        let mut entries = BTreeMap::new();
        let mut edn = String::from("{:proposals [");
        for (name, proposal) in &proposals {
            let status = if self.externs.contains_spec(name) {
                ":existing"
            } else if proposal.unknown.is_empty()
                && let Some(conversion) = self.specs.conversions.get(*name)
                && let Some(namespace) = self.specs.standalone_namespace(name, &proposal.path)
            {
                entries.insert(*name, (namespace, conversion));
                ":generated"
            } else {
                "nil"
            };
            edn.push_str("\n ");
            proposal.write_edn(&mut edn, status);
        }
        let externs = encode_externs(&entries)?;
        write!(edn, "]\n :externs {}}}\n", externs.trim_end()).unwrap();
        Ok(ContractProposals {
            report,
            edn: Some(edn),
            externs: Some(externs),
        })
    }
}

impl SpecContractProposal {
    fn write_edn(&self, edn: &mut String, status: &str) {
        let edn_string = |s: &str| serde_json::to_string(s).expect("serialize string");
        let describe = |s: Option<&str>| s.map_or_else(|| "nil".into(), edn_string);
        let source = edn_string(&self.path);
        let form = edn_string(&self.form);
        let input = describe(self.input.as_deref());
        let result = describe(self.result.as_deref());
        let line = self.span.start.line;
        let column = self.span.start.column;
        let unknown = self
            .unknown
            .iter()
            .map(|s| edn_string(s))
            .collect::<Vec<_>>()
            .join(" ");
        write!(edn, "{{:name {} :source {{:path {source} :line {line} :column {column} :form {form}}} :input {input} :result {result} :unknown [{unknown}] :extern {status}}}", self.name).unwrap();
    }
}

fn display_type(ty: &Type) -> Option<String> {
    // The existing encoder bounds depth and expanded size before displaying
    // a potentially shared spec graph. Unknown components fail encoding.
    SnapshotWriter::new().ty(ty).ok()?;
    Some(ty.to_string())
}

fn encode_externs(entries: &BTreeMap<&str, (&str, &SpecConversion)>) -> Result<String, String> {
    let mut out = SnapshotWriter::new();
    for (index, (name, (namespace, conversion))) in entries.iter().enumerate() {
        if index != 0 {
            out.write_str("\n ").map_err(|err| err.to_string())?;
        }
        write!(out, "{name} ").map_err(|err| err.to_string())?;
        out.spec(namespace, conversion)
            .map_err(|err| format!("Cannot export spec {name}: {err}"))?;
    }
    let edn = out.finish().map_err(|err| err.to_string())?;
    let path = "<proposed externs>";
    let mut snapshot = FileCache::default();
    snapshot.update(&edn, path, FileKind::Externs);
    if let Some(diagnostic) = snapshot.parse_error {
        return Err(diagnostic.message);
    }
    let parsed = Model::externs(&snapshot.forms, path, &edn, &Globals::default());
    if let Some(diagnostic) = parsed.diagnostics.first() {
        return Err(diagnostic.message.clone());
    }
    if parsed.extern_count() != entries.len()
        || entries
            .iter()
            .any(|(name, spec)| parsed.extern_spec(name) != Some(*spec))
    {
        return Err("Generated EDN did not preserve the named spec conversions".into());
    }
    Ok(edn)
}

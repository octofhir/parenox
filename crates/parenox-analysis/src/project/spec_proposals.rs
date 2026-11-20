//! Derive reviewable named spec contracts from the current project snapshot.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use super::{FileCache, FileKind, ProjectReport, Session, SnapshotMode, Source};
use crate::checker::{ExternCache, Globals, Model, SnapshotWriter, SpecConversion};
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
    /// The literal initializer of a live Var, as written in this snapshot.
    pub initial_table: Option<String>,
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
            .map(|(proposal, _)| proposal)
    }

    fn describe_spec(
        &self,
        path: &str,
        name: &str,
        standard_spec: bool,
    ) -> Option<(SpecContractProposal, Option<&str>)> {
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
        let input_known = !self.specs.unchecked_input(name) && input.is_some();
        let partial = conversion.is_some_and(SpecConversion::has_unchecked_input);
        let bundle = conversion.and_then(SpecConversion::bundle_fields);
        let namespace = if declarations.has_namespace_declaration() {
            self.specs.standalone_namespace(name, path)
        } else {
            None
        };
        let children_available = bundle.is_none_or(|fields| {
            fields.values().all(|field| {
                !self.externs.declares_spec(&field.registration)
                    || namespace
                        .zip(field.conversion.as_ref())
                        .is_some_and(|expected| {
                            self.externs.extern_spec(&field.registration) == Some(expected)
                        })
            })
        });
        let qualified = declarations.resolve_spec_keyword(name).as_deref() == Some(name);
        let export_namespace = namespace.filter(|_| {
            result.is_some()
                && qualified
                && children_available
                && (input_known || partial || bundle.is_some())
        });
        let mut unknown = Vec::new();
        if !input_known {
            unknown.push("Input domain");
        } else if result.is_none() {
            unknown.push("Full validation behavior of the input projection");
        }
        if result.is_none() {
            unknown.push("Conformed result");
        } else if namespace.is_none() {
            unknown.push("Standalone registration");
        }
        if !qualified {
            unknown.push("Qualified registration name");
        }
        if bundle.is_some() {
            unknown.push("Registrations for additional qualified keys");
        }
        if !children_available {
            unknown.push("Bundle with an extern child that does not match its source registration");
        }
        if self.externs.declares_spec(name) && self.externs.extern_spec(name).is_none() {
            unknown.push("Existing extern could not be resolved");
        }
        let initial_table = match conversion {
            Some(SpecConversion::GuardedMap(_)) => {
                unknown.push("Exception classes from custom lookups");
                None
            }
            Some(SpecConversion::LiveLookupMap(projection)) => {
                unknown.push("Runtime value of the lookup Var");
                unknown.push("Exception classes from live lookups");
                declarations
                    .live_lookup_initializer_span(
                        &file.forms,
                        &file.source,
                        &projection.var,
                        span.start.offset,
                    )
                    .and_then(|span| file.source.get(span.start.offset..span.end.offset))
                    .map(str::to_owned)
            }
            _ => None,
        };
        if !standard_spec {
            unknown.push("Behavior of the project's clojure.spec.alpha");
        }
        let proposal = SpecContractProposal {
            name: name.into(),
            path: path.into(),
            span,
            form: file.source.get(span.start.offset..span.end.offset)?.into(),
            input,
            result,
            unknown,
            initial_table,
        };
        Some((proposal, export_namespace))
    }

    /// Generate proposals and extern declarations without editing the source or
    /// supplied externs. Supported bundles reuse matching named extern children.
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
        for (name, (proposal, namespace)) in &proposals {
            let status = if self.externs.declares_spec(name) {
                if self.externs.extern_spec(name).is_some() {
                    ":existing"
                } else {
                    "nil"
                }
            } else if let Some(namespace) = namespace
                && let Some(conversion) = self.specs.conversions.get(*name)
            {
                entries.insert(*name, (*namespace, conversion));
                if let Some(fields) = conversion.bundle_fields() {
                    entries.extend(
                        fields
                            .values()
                            .filter(|field| !self.externs.declares_spec(&field.registration))
                            .filter_map(|field| {
                                Some((
                                    field.registration.as_str(),
                                    (*namespace, field.conversion.as_ref()?),
                                ))
                            }),
                    );
                }
                ":generated"
            } else {
                "nil"
            };
            edn.push_str("\n ");
            proposal.write_edn(&mut edn, status);
        }
        let externs = encode_externs(&entries, &self.externs)?;
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
        write!(edn, "{{:name {} :source {{:path {source} :line {line} :column {column} :form {form}}} :input {input} :result {result} :unknown [{unknown}] :extern {status}", self.name).unwrap();
        if let Some(table) = &self.initial_table {
            write!(edn, " :initial-table {}", edn_string(table)).unwrap();
        }
        edn.push('}');
    }
}

fn display_type(ty: &Type) -> Option<String> {
    // The existing encoder bounds depth and expanded size before displaying
    // a potentially shared spec graph. Unknown components fail encoding.
    SnapshotWriter::new().ty(ty).ok()?;
    Some(ty.to_string())
}

fn encode_externs(
    entries: &BTreeMap<&str, (&str, &SpecConversion)>,
    existing: &ExternCache,
) -> Result<String, String> {
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
    let file = parsed
        .extern_file()
        .ok_or("Generated EDN has no extern file")?;
    let merged = existing.with_generated_file(file);
    if let Some(diagnostic) = merged.diagnostics(path).first() {
        return Err(diagnostic.message.clone());
    }
    if parsed.extern_count() != entries.len()
        || entries
            .iter()
            .any(|(name, spec)| merged.extern_spec(name) != Some(*spec))
    {
        return Err("Generated EDN did not preserve the named spec conversions".into());
    }
    Ok(edn)
}

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;

use crate::checker::{Declarations, Globals, Model, SpecCache, SpecFingerprint, SpecSource};
use crate::reader::{self, Form, Position, Span};
use crate::{Diagnostic, Report, Stats};

#[derive(Clone, Copy)]
pub struct Source<'a> {
    pub path: &'a str,
    pub source: &'a str,
}

#[derive(Debug, Serialize)]
pub struct ProjectReport {
    pub version: u32,
    pub ok: bool,
    pub complete: bool,
    pub files: Vec<Report>,
    pub stats: Stats,
}

#[derive(Default)]
struct FileCache {
    source: Arc<str>,
    source_hash: u64,
    parsed: bool,
    kind: FileKind,
    forms: Vec<Form>,
    parse_error: Option<Diagnostic>,
    declarations: Option<Declarations>,
    spec_fingerprint: Option<Arc<SpecFingerprint>>,
    model: Option<Model>,
    interface: String,
    functions: BTreeMap<String, CheckedFunction>,
}

struct CheckedFunction {
    source: Arc<str>,
    ranges: Vec<Span>,
    diagnostics: Vec<Diagnostic>,
}

impl CheckedFunction {
    fn reusable(&self, source: &str, ranges: &[Span]) -> bool {
        // Diagnostics and call-chain notes contain absolute source positions.
        if !self.diagnostics.is_empty() && self.ranges != ranges {
            return false;
        }
        self.ranges.len() == ranges.len()
            && self.ranges.iter().zip(ranges).all(|(old, new)| {
                self.source[old.start.offset..old.end.offset]
                    == source[new.start.offset..new.end.offset]
            })
    }
}

#[derive(Default, PartialEq, Eq, Clone, Copy)]
enum FileKind {
    #[default]
    Source,
    Externs,
}

impl FileCache {
    fn update(&mut self, source: &str, path: &str, kind: FileKind) -> bool {
        if self.parsed && self.source.as_ref() == source && self.kind == kind {
            return false;
        }
        self.source = Arc::from(source);
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        self.source_hash = hasher.finish();
        self.parsed = true;
        self.kind = kind;
        self.model = None;
        self.parse_error = None;
        self.declarations = None;
        let conditionals = if kind == FileKind::Source && path.ends_with(".cljc") {
            reader::Conditionals::Select(":clj")
        } else {
            reader::Conditionals::Reject
        };
        match reader::read_with(source, conditionals) {
            Ok(forms) => {
                if kind == FileKind::Source {
                    let declarations = Declarations::read(&forms);
                    let fingerprint = declarations.spec_fingerprint(source);
                    if self.spec_fingerprint.as_deref() != fingerprint.as_ref() {
                        self.spec_fingerprint = fingerprint.map(Arc::new);
                    }
                    self.declarations = Some(declarations);
                } else {
                    self.spec_fingerprint = None;
                }
                self.forms = forms;
            }
            Err(error) => {
                self.forms.clear();
                self.spec_fingerprint = None;
                self.parse_error = Some(error.into());
            }
        }
        true
    }

    fn start_span(&self) -> Span {
        self.forms.first().map_or(
            Span {
                start: Position {
                    offset: 0,
                    line: 1,
                    column: 1,
                },
                end: Position {
                    offset: 0,
                    line: 1,
                    column: 1,
                },
            },
            |form| form.span,
        )
    }
}

/// In-memory source snapshots. No checked project code is executed.
#[derive(Default)]
pub struct Session {
    files: BTreeMap<String, FileCache>,
    specs: SpecCache,
}

impl Session {
    pub fn check(&mut self, path: &str, source: &str) -> Report {
        let mut project = self.check_project(&[Source { path, source }], None);
        let mut report = project.files.remove(0);
        report.stats.elapsed_us = project.stats.elapsed_us;
        report
    }

    /// A complete project snapshot plus one optional EDN extern file.
    /// Removed sources are absent from the new environment; cached declarations never leak in.
    pub fn check_project(
        &mut self,
        sources: &[Source<'_>],
        externs: Option<Source<'_>>,
    ) -> ProjectReport {
        let started = Instant::now();
        let mut entries = Vec::new();
        let mut reports = Vec::new();
        let mut paths = BTreeSet::new();
        let count = sources.len();
        for (index, source) in sources.iter().copied().chain(externs).enumerate() {
            let kind = if index < count {
                FileKind::Source
            } else {
                FileKind::Externs
            };
            let mut cache = self.files.remove(source.path).unwrap_or_default();
            let parsed = cache.update(source.source, source.path, kind);
            let mut diagnostics: Vec<_> = cache.parse_error.iter().cloned().collect();
            if !paths.insert(source.path) {
                diagnostics.push(Diagnostic::new(
                    "PNX2790",
                    cache.start_span(),
                    "Duplicate source path",
                ));
            }
            reports.push(Report {
                version: 1,
                path: source.path.into(),
                source_hash: format!("{:016x}", cache.source_hash),
                ok: false,
                complete: false,
                diagnostics,
                stats: Stats {
                    parsed_files: usize::from(parsed),
                    reused_parses: usize::from(!parsed),
                    ..Stats::default()
                },
            });
            entries.push((source, cache, parsed));
        }
        let mut namespaces = BTreeMap::new();
        for (index, (_, cache, _)) in entries[..count].iter().enumerate() {
            if let Some(declarations) = &cache.declarations
                && namespaces
                    .insert(declarations.namespace().to_owned(), index)
                    .is_some()
            {
                reports[index].diagnostics.push(Diagnostic::new(
                    "PNX2790",
                    cache.start_span(),
                    format!("Duplicate namespace {}", declarations.namespace()),
                ));
            }
        }
        let mut pending = vec![0; count];
        let mut consumers = vec![Vec::new(); count];
        for (index, (_, cache, _)) in entries[..count].iter().enumerate() {
            if let Some(declarations) = &cache.declarations {
                for dependency in &declarations.dependencies {
                    if let Some(&owner) = namespaces.get(dependency) {
                        pending[index] += 1;
                        consumers[owner].push(index);
                    }
                }
            }
        }
        let mut ready: VecDeque<_> = (0..count).filter(|&i| pending[i] == 0).collect();
        let mut built = vec![false; count];
        let mut order = Vec::with_capacity(count);
        while let Some(index) = ready.pop_front() {
            built[index] = true;
            order.push(index);
            for &consumer in &consumers[index] {
                pending[consumer] -= 1;
                if pending[consumer] == 0 {
                    ready.push_back(consumer);
                }
            }
        }
        for index in 0..count {
            if !built[index] {
                reports[index].diagnostics.push(Diagnostic::new(
                    "PNX9005",
                    entries[index].1.start_span(),
                    "Namespace dependency cycle; this file could not be checked",
                ));
            }
        }
        self.specs.update(
            order
                .iter()
                .filter_map(|&index| {
                    let (source, cache, _) = &entries[index];
                    Some(SpecSource {
                        path: source.path,
                        declarations: cache.declarations.as_ref()?,
                        fingerprint: cache.spec_fingerprint.as_ref()?,
                    })
                })
                .collect(),
        );
        let mut globals = Globals::with_specs(self.specs.types.clone());
        for namespace in namespaces.keys() {
            globals.register_namespace(namespace);
        }
        for index in order {
            let (_, cache, _) = &mut entries[index];
            if let Some(declarations) = &cache.declarations {
                if cache
                    .model
                    .as_ref()
                    .is_none_or(|model| !model.external_specs_current(&globals))
                {
                    cache.model = Some(Model::build(declarations, &cache.forms, &globals));
                }
                cache.model.as_ref().unwrap().publish(&mut globals);
            }
        }
        if externs.is_some() {
            let (_, cache, _) = &mut entries[count];
            if cache.parse_error.is_none() {
                if cache
                    .model
                    .as_ref()
                    .is_none_or(|model| !model.external_specs_current(&globals))
                {
                    cache.model = Some(Model::externs(&cache.forms, &globals));
                }
                cache.model.as_ref().unwrap().publish_externs(&mut globals);
            }
        }
        let mut totals = Stats::default();
        for (index, (source, mut cache, changed)) in entries.into_iter().enumerate() {
            let file_started = Instant::now();
            let report = &mut reports[index];
            if (index >= count || built[index])
                && let Some(model) = &cache.model
            {
                if let Some(diagnostics) = self.specs.diagnostics.get(source.path) {
                    report.diagnostics.extend(diagnostics.iter().cloned());
                }
                report.diagnostics.extend(model.diagnostics.iter().cloned());
                let interface = model.interface_key(&globals);
                if !changed && cache.interface == interface {
                    report.stats.reused_functions = cache.functions.len();
                    report.diagnostics.extend(
                        cache
                            .functions
                            .values()
                            .flat_map(|function| function.diagnostics.iter().cloned()),
                    );
                } else {
                    let previous = std::mem::take(&mut cache.functions);
                    for function in model.functions.values().filter(|f| !f.inferred) {
                        let ranges = model.function_ranges(function);
                        let diagnostics = if let Some(cached) =
                            previous.get(&function.name).filter(|cached| {
                                cache.interface == interface
                                    && cached.reusable(source.source, &ranges)
                            }) {
                            report.stats.reused_functions += 1;
                            cached.diagnostics.clone()
                        } else {
                            report.stats.checked_functions += 1;
                            model.check_function(function, source.source, &globals)
                        };
                        report.diagnostics.extend(diagnostics.iter().cloned());
                        cache.functions.insert(
                            function.name.clone(),
                            CheckedFunction {
                                source: cache.source.clone(),
                                ranges,
                                diagnostics,
                            },
                        );
                    }
                }
                cache.interface = interface;
            }
            report.diagnostics.sort_by(|a, b| {
                (a.span.start.offset, &a.code).cmp(&(b.span.start.offset, &b.code))
            });
            report.ok = report.diagnostics.is_empty();
            report.complete = !report
                .diagnostics
                .iter()
                .any(|d| d.code.starts_with("PNX9"));
            report.stats.elapsed_us = file_started.elapsed().as_micros() as u64;
            totals.checked_functions += report.stats.checked_functions;
            totals.reused_functions += report.stats.reused_functions;
            totals.parsed_files += report.stats.parsed_files;
            totals.reused_parses += report.stats.reused_parses;
            self.files.insert(source.path.into(), cache);
        }
        totals.elapsed_us = started.elapsed().as_micros() as u64;
        ProjectReport {
            version: 1,
            ok: reports.iter().all(|r| r.ok),
            complete: reports.iter().all(|r| r.complete),
            files: reports,
            stats: totals,
        }
    }
}

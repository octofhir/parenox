use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Instant;

use serde::Serialize;

use crate::checker::{Globals, Model};
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
    source: String,
    parsed: bool,
    forms: Vec<Form>,
    parse_error: Option<Diagnostic>,
    namespace: String,
    dependencies: Vec<String>,
    model: Option<Model>,
    build_key: String,
    interface: String,
    functions: BTreeMap<String, (String, Vec<Diagnostic>)>,
}

impl FileCache {
    fn update(&mut self, source: &str, path: &str) -> bool {
        if self.parsed && self.source == source {
            return false;
        }
        self.source.clear();
        self.source.push_str(source);
        self.parsed = true;
        self.model = None;
        self.parse_error = None;
        let conditionals = if path.ends_with(".cljc") {
            reader::Conditionals::Select(":clj")
        } else {
            reader::Conditionals::Reject
        };
        match reader::read_with(source, conditionals) {
            Ok(forms) => {
                let context = Model::context(&forms);
                self.namespace = context.ns;
                self.dependencies = context.dependencies;
                self.forms = forms;
            }
            Err(error) => {
                self.forms.clear();
                self.namespace.clear();
                self.dependencies.clear();
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
        for source in sources.iter().copied().chain(externs) {
            let mut cache = self.files.remove(source.path).unwrap_or_default();
            let parsed = cache.update(source.source, source.path);
            let mut hasher = DefaultHasher::new();
            source.source.hash(&mut hasher);
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
                source_hash: format!("{:016x}", hasher.finish()),
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
        let count = sources.len();
        let mut namespaces = BTreeMap::new();
        for (index, (_, cache, _)) in entries[..count].iter().enumerate() {
            if cache.parse_error.is_none()
                && namespaces.insert(cache.namespace.clone(), index).is_some()
            {
                reports[index].diagnostics.push(Diagnostic::new(
                    "PNX2790",
                    cache.start_span(),
                    format!("Duplicate namespace {}", cache.namespace),
                ));
            }
        }
        let mut pending = vec![0; count];
        let mut consumers = vec![Vec::new(); count];
        for (index, (_, cache, _)) in entries[..count].iter().enumerate() {
            for dependency in &cache.dependencies {
                if let Some(&owner) = namespaces.get(dependency) {
                    pending[index] += 1;
                    consumers[owner].push(index);
                }
            }
        }
        let mut ready: VecDeque<_> = (0..count).filter(|&i| pending[i] == 0).collect();
        let mut globals = Globals::default();
        for namespace in namespaces.keys() {
            globals.register_namespace(namespace);
        }
        let mut built = vec![false; count];
        while let Some(index) = ready.pop_front() {
            built[index] = true;
            let (_, cache, _) = &mut entries[index];
            let key = globals.dependency_key(&cache.dependencies);
            if cache.parse_error.is_none() {
                if cache.model.is_none() || cache.build_key != key {
                    cache.model = Some(Model::build(&cache.forms, &globals));
                    cache.build_key = key;
                }
                cache.model.as_ref().unwrap().publish(&mut globals);
            }
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
        if externs.is_some() {
            let (_, cache, _) = &mut entries[count];
            if cache.parse_error.is_none() {
                cache.model = Some(Model::externs(&cache.forms, &globals));
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
                report.diagnostics.extend(model.diagnostics.iter().cloned());
                let interface = model.interface_key(&globals);
                if !changed && cache.interface == interface {
                    report.stats.reused_functions = cache.functions.len();
                    report.diagnostics.extend(
                        cache
                            .functions
                            .values()
                            .flat_map(|(_, ds)| ds.iter().cloned()),
                    );
                } else {
                    let previous = std::mem::take(&mut cache.functions);
                    for function in model.functions.values().filter(|f| !f.inferred) {
                        let key = model.function_key(function, source.source);
                        let errors = if let Some((_, errors)) = previous
                            .get(&function.name)
                            .filter(|(old, _)| cache.interface == interface && old == &key)
                        {
                            report.stats.reused_functions += 1;
                            errors.clone()
                        } else {
                            report.stats.checked_functions += 1;
                            model.check_function(function, source.source, &globals)
                        };
                        report.diagnostics.extend(errors.iter().cloned());
                        cache.functions.insert(function.name.clone(), (key, errors));
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

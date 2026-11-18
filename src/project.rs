mod extern_export;
mod navigation;

pub use navigation::{Definition, VarInfo};

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use serde::Serialize;

use crate::checker::{
    Declarations, ExternCache, Globals, InterfaceKey, Model, SpecCache, SpecFingerprint, SpecSource,
};
use crate::reader::{self, Form, Position, Span};
use crate::suppressions::{MARKER, Suppressions};
use crate::{Diagnostic, ExternChangeKind, ExternDiff, ExternSnapshot, Report, Stats};

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

#[derive(Debug)]
pub struct ExternExport {
    pub report: ProjectReport,
    /// Standalone EDN, present only after a successful and complete check.
    pub edn: Option<String>,
}

/// Compares the new extern promises to the old ones with independent spec
/// environments. An invalid snapshot never turns missing declarations into removals.
pub fn compare_externs(before: ExternSnapshot<'_>, after: ExternSnapshot<'_>) -> ExternDiff {
    let mut old = Session::default();
    let mut new = Session::default();
    let before = old.check_snapshot(
        before.sources,
        before.externs,
        &[],
        SnapshotMode::ExternSurface,
    );
    let after = new.check_snapshot(
        after.sources,
        after.externs,
        &[],
        SnapshotMode::ExternSurface,
    );
    let valid = before.ok && before.complete && after.ok && after.complete;
    let changes = if valid {
        new.externs.changes_from(&old.externs)
    } else {
        Vec::new()
    };
    let complete = valid
        && !changes
            .iter()
            .any(|change| change.kind == ExternChangeKind::Unproven);
    let compatible = complete
        && !changes.iter().any(|change| {
            matches!(
                change.kind,
                ExternChangeKind::Removed
                    | ExternChangeKind::Breaking
                    | ExternChangeKind::ValueChanged
            )
        });
    ExternDiff {
        version: 1,
        compatible,
        complete,
        before,
        after,
        changes,
    }
}

#[derive(Default)]
struct FileCache {
    source: Arc<str>,
    source_hash: u64,
    snapshot: u64,
    parsed: bool,
    kind: FileKind,
    forms: Vec<Form>,
    parse_error: Option<Diagnostic>,
    suppressions: Suppressions,
    declarations: Option<Declarations>,
    known_var_surface: OnceLock<bool>,
    navigation_prefix: Option<Box<NavigationPrefix>>,
    spec_fingerprint: Option<Arc<SpecFingerprint>>,
    model: Option<Model>,
    interface: Option<InterfaceKey>,
    checked_source: Arc<str>,
    functions: BTreeMap<String, CheckedFunction>,
}

struct CheckedFunction {
    ranges: Vec<Span>,
    diagnostics: Vec<Diagnostic>,
}

struct NavigationPrefix {
    forms: Vec<Form>,
    declarations: Declarations,
}

impl CheckedFunction {
    fn reusable(&self, previous_source: &str, source: &str, ranges: &[Span]) -> bool {
        // Diagnostics and call-chain notes contain absolute source positions.
        if !self.diagnostics.is_empty() && self.ranges != ranges {
            return false;
        }
        self.ranges.len() == ranges.len()
            && self.ranges.iter().zip(ranges).all(|(old, new)| {
                previous_source[old.start.offset..old.end.offset]
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum SnapshotMode {
    Check,
    ValidateExterns,
    ExternSurface,
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
        self.suppressions = Suppressions::default();
        self.declarations = None;
        self.known_var_surface.take();
        self.navigation_prefix = None;
        let conditionals = if kind == FileKind::Source && path.ends_with(".cljc") {
            reader::Conditionals::Select(":clj")
        } else {
            reader::Conditionals::Reject
        };
        let (mut read, read_error) = reader::read_for_check(
            source,
            conditionals,
            (kind == FileKind::Source && source.contains(MARKER)).then_some(MARKER),
        );
        let parsed = match read_error {
            Some(error) => Err(error),
            None if kind == FileKind::Source => {
                let declarations = Declarations::read(&read.forms);
                declarations
                    .validate_keyword_aliases(source, &read.auto_keywords)
                    .map(|()| Some(declarations))
            }
            None => read.auto_keywords.first().map_or(Ok(None), |&span| {
                Err(reader::ReadError {
                    span,
                    message: "Auto-resolved keywords are invalid in EDN externs".into(),
                })
            }),
        };
        match parsed {
            Ok(declarations) => {
                let fingerprint = declarations
                    .as_ref()
                    .and_then(|declarations| declarations.spec_fingerprint(source));
                if self.spec_fingerprint.as_deref() != fingerprint.as_ref() {
                    self.spec_fingerprint = fingerprint.map(Arc::new);
                }
                self.declarations = declarations;
                self.suppressions = Suppressions::read(source, &read.forms, read.comments);
                self.forms = read.forms;
            }
            Err(error) => {
                self.forms.clear();
                if kind == FileKind::Source {
                    read.forms
                        .retain(|form| form.span.end.offset <= error.span.start.offset);
                    if !read.forms.is_empty() {
                        self.navigation_prefix = Some(Box::new(NavigationPrefix {
                            declarations: Declarations::read(&read.forms),
                            forms: read.forms,
                        }));
                    }
                }
                self.spec_fingerprint = None;
                self.parse_error = Some(error.into());
            }
        }
        true
    }

    fn navigation_source(&self) -> Option<(&Declarations, &[Form])> {
        if let Some(declarations) = &self.declarations {
            Some((declarations, &self.forms))
        } else {
            let prefix = self.navigation_prefix.as_deref()?;
            Some((&prefix.declarations, &prefix.forms))
        }
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

    fn public_literals(&self) -> impl Iterator<Item = (&str, Span, Result<String, &'static str>)> {
        self.model
            .as_ref()
            .zip(self.declarations.as_ref())
            .into_iter()
            .flat_map(|(model, declarations)| {
                model.public_literals(declarations, &self.forms, &self.source)
            })
    }
}

/// In-memory source snapshots. No checked project code is executed.
#[derive(Default)]
pub struct Session {
    files: BTreeMap<String, FileCache>,
    specs: SpecCache,
    externs: ExternCache,
    snapshot: u64,
}

impl Session {
    /// Discard cached data for a path. The next check uses the supplied source.
    pub fn forget_file(&mut self, path: &str) {
        self.files.remove(path);
    }

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
        self.check_project_with_externs(sources, externs.as_slice())
    }

    /// Checks a complete snapshot with any number of separate EDN extern files.
    /// A qualified name declared in multiple extern files is not published.
    pub fn check_project_with_externs(
        &mut self,
        sources: &[Source<'_>],
        externs: &[Source<'_>],
    ) -> ProjectReport {
        self.check_snapshot(sources, externs, &[], SnapshotMode::Check)
    }

    /// Checks library source without trusting the supplied extern promises and
    /// verifies each promise against a checked public source declaration.
    /// Validation succeeds only when the resulting report is both ok and complete.
    pub fn validate_externs(
        &mut self,
        sources: &[Source<'_>],
        externs: &[Source<'_>],
    ) -> ProjectReport {
        self.validate_externs_with_dependencies(sources, externs, &[])
    }

    /// Validates the selected externs while trusting separate dependency externs.
    /// If a qualified name occurs in both groups, neither declaration is used.
    pub fn validate_externs_with_dependencies(
        &mut self,
        sources: &[Source<'_>],
        externs: &[Source<'_>],
        dependencies: &[Source<'_>],
    ) -> ProjectReport {
        self.check_snapshot(
            sources,
            externs,
            dependencies,
            SnapshotMode::ValidateExterns,
        )
    }

    fn check_snapshot(
        &mut self,
        sources: &[Source<'_>],
        externs: &[Source<'_>],
        dependencies: &[Source<'_>],
        mode: SnapshotMode,
    ) -> ProjectReport {
        let started = Instant::now();
        self.snapshot += 1;
        let mut entries = Vec::new();
        let mut reports = Vec::new();
        let mut paths = BTreeSet::new();
        let count = sources.len();
        let checked_end = count + externs.len();
        for (index, source) in sources
            .iter()
            .chain(externs)
            .chain(dependencies)
            .copied()
            .enumerate()
        {
            let kind = if index < count {
                FileKind::Source
            } else {
                FileKind::Externs
            };
            let mut cache = self.files.remove(source.path).unwrap_or_default();
            let parsed = cache.update(source.source, source.path, kind);
            let mut diagnostics: Vec<_> = cache.parse_error.iter().cloned().collect();
            let unique = paths.insert(source.path);
            cache.snapshot = if unique && index < count {
                self.snapshot
            } else {
                0
            };
            if !unique {
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
                suppressed: Vec::new(),
                stats: Stats {
                    parsed_files: usize::from(parsed),
                    reused_parses: usize::from(!parsed),
                    ..Stats::default()
                },
            });
            entries.push((source, cache, parsed));
        }
        let mut namespaces = BTreeMap::new();
        let mut ambiguous = BTreeSet::new();
        for (index, (_, cache, _)) in entries[..count].iter().enumerate() {
            if let Some(declarations) = &cache.declarations
                && namespaces
                    .insert(declarations.namespace().to_owned(), index)
                    .is_some()
            {
                ambiguous.insert(declarations.namespace().to_owned());
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
        if mode != SnapshotMode::ExternSurface {
            for index in order {
                let (source, cache, _) = &mut entries[index];
                if let Some(declarations) = &cache.declarations {
                    if cache
                        .model
                        .as_mut()
                        .is_none_or(|model| !model.refresh_external_inputs(&globals))
                    {
                        cache.model = Some(Model::build(
                            declarations,
                            &cache.forms,
                            &cache.source,
                            source.path,
                            &globals,
                        ));
                    }
                    cache.model.as_ref().unwrap().publish(&mut globals);
                }
            }
        }
        for (source, cache, _) in &mut entries[count..] {
            if cache.parse_error.is_none()
                && cache
                    .model
                    .as_mut()
                    .is_none_or(|model| !model.refresh_external_inputs(&globals))
            {
                cache.model = Some(Model::externs(
                    &cache.forms,
                    source.path,
                    &cache.source,
                    &globals,
                ));
            }
        }
        self.externs.update(
            entries[count..]
                .iter()
                .filter_map(|(_, cache, _)| cache.model.as_ref()?.extern_file()),
        );
        let source_literals =
            if mode == SnapshotMode::ValidateExterns && self.externs.has_literals() {
                entries[..count]
                    .iter()
                    .zip(&built)
                    .filter(|(_, built)| **built)
                    .flat_map(|((source, cache, _), _)| {
                        cache
                            .public_literals()
                            .map(move |(name, _, value)| (name.to_owned(), (source.path, value)))
                    })
                    .collect()
            } else {
                BTreeMap::new()
            };
        if mode == SnapshotMode::Check {
            self.externs.publish(&mut globals);
        } else if mode == SnapshotMode::ValidateExterns && !dependencies.is_empty() {
            let mut trusted_paths: BTreeSet<_> =
                dependencies.iter().map(|file| file.path).collect();
            // Even an invalid snapshot must not make a checked path trusted.
            for file in externs {
                trusted_paths.remove(file.path);
            }
            self.externs
                .publish_dependencies(&trusted_paths, &mut globals);
        }
        if mode == SnapshotMode::Check {
            for (index, (_, file, _)) in entries[..count].iter().enumerate() {
                if let (Some(declarations), Some(model)) = (&file.declarations, &file.model) {
                    reports[index]
                        .diagnostics
                        .extend(declarations.check_untyped_references(
                            &file.forms,
                            model,
                            |namespace, local| {
                                let Some(&index) = namespaces.get(namespace) else {
                                    return false;
                                };
                                let (_, provider, _) = &entries[index];
                                built[index]
                                    && provider.snapshot == self.snapshot
                                    && !ambiguous.contains(namespace)
                                    && provider.declarations.as_ref().is_some_and(|declarations| {
                                        declarations
                                            .source_var(&provider.forms, local, true)
                                            .is_none()
                                            && *provider.known_var_surface.get_or_init(|| {
                                                declarations.has_known_var_surface(&provider.forms)
                                            })
                                    })
                            },
                        ));
                }
            }
        }
        let mut totals = Stats::default();
        let mut report_ordinals = BTreeMap::<&str, usize>::new();
        for (index, (source, mut cache, changed)) in entries.into_iter().enumerate() {
            let file_started = Instant::now();
            let report = &mut reports[index];
            let specs_only = mode == SnapshotMode::ExternSurface && index < count;
            if specs_only {
                if let Some(declarations) = &cache.declarations {
                    report
                        .diagnostics
                        .extend_from_slice(declarations.diagnostics());
                }
                if let Some(diagnostics) = self.specs.diagnostics.get(source.path) {
                    report.diagnostics.extend_from_slice(diagnostics);
                }
            } else if (index >= count || built[index])
                && let Some(model) = &cache.model
            {
                if let Some(diagnostics) = self.specs.diagnostics.get(source.path) {
                    report.diagnostics.extend(diagnostics.iter().cloned());
                }
                report.diagnostics.extend(model.diagnostics.iter().cloned());
                if index >= count {
                    report
                        .diagnostics
                        .extend_from_slice(self.externs.diagnostics(source.path));
                    if mode == SnapshotMode::ValidateExterns
                        && index < checked_end
                        && let Some(file) = model.extern_file()
                    {
                        report.diagnostics.extend(self.externs.validate_file(
                            file,
                            &globals,
                            &source_literals,
                        ));
                    }
                }
                let interface = model.interface_key(&globals);
                let changes = cache
                    .interface
                    .as_ref()
                    .map(|previous| interface.changes(previous));
                if !changed
                    && changes
                        .as_ref()
                        .is_some_and(|changes| changes.is_unchanged())
                {
                    report.stats.reused_functions = cache.functions.len();
                    report.diagnostics.extend(
                        cache
                            .functions
                            .values()
                            .flat_map(|function| function.diagnostics.iter().cloned()),
                    );
                } else {
                    for function in model.functions.values().filter(|f| !f.inferred()) {
                        let ranges = model.function_ranges(function);
                        if let Some(cached) =
                            cache.functions.get_mut(&function.name).filter(|cached| {
                                cached.reusable(&cache.checked_source, source.source, &ranges)
                                    && changes.as_ref().is_some_and(|changes| {
                                        // Cached messages can contain the original binder names.
                                        changes.allows_reuse(
                                            model,
                                            function,
                                            cached.diagnostics.is_empty(),
                                        )
                                    })
                            })
                        {
                            report.stats.reused_functions += 1;
                            report
                                .diagnostics
                                .extend(cached.diagnostics.iter().cloned());
                            cached.ranges = ranges;
                        } else {
                            report.stats.checked_functions += 1;
                            let diagnostics =
                                model.check_function(function, source.source, &globals);
                            report.diagnostics.extend(diagnostics.iter().cloned());
                            cache.functions.insert(
                                function.name.clone(),
                                CheckedFunction {
                                    ranges,
                                    diagnostics,
                                },
                            );
                        }
                    }
                    let active = report.stats.checked_functions + report.stats.reused_functions;
                    if cache.functions.len() != active {
                        cache.functions.retain(|name, _| {
                            model
                                .functions
                                .get(name)
                                .is_some_and(|function| !function.inferred())
                        });
                    }
                    cache.checked_source = cache.source.clone();
                }
                cache.interface = Some(interface);
            }
            if !specs_only {
                report
                    .diagnostics
                    .extend(cache.suppressions.invalid.iter().cloned());
            }
            if !report.diagnostics.is_empty() || (!specs_only && cache.suppressions.has_rules()) {
                // A caller can supply duplicate paths in an invalid snapshot. Their
                // reports must not share IDs even when the diagnostics are identical.
                let ordinal = report_ordinals.entry(source.path).or_default();
                identify_diagnostics(source.path, *ordinal, &mut report.diagnostics);
                if !specs_only {
                    let mut unused = cache.suppressions.apply(report);
                    if !unused.is_empty() {
                        // Only this step emits PNX9007; these IDs cannot collide
                        // with IDs assigned before filtering.
                        identify_diagnostics(source.path, *ordinal, &mut unused);
                        report.diagnostics.append(&mut unused);
                        report.diagnostics.sort_by(diagnostic_order);
                    }
                }
                *ordinal += 1;
            }
            report.ok = report.diagnostics.is_empty();
            report.complete = report.suppressed.is_empty()
                && !report
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

fn identify_diagnostics(path: &str, report: usize, diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(diagnostic_order);

    // Assign IDs after merging cached and fresh diagnostics. Stable sorting keeps
    // the original analysis order among diagnostics with identical spans and rules.
    let mut previous = None;
    let mut ordinal = 0;
    for diagnostic in diagnostics {
        let (start, end) = (diagnostic.span.start.offset, diagnostic.span.end.offset);
        let code = diagnostic.code.as_str();
        let current = (start, end, code);
        if previous != Some(current) {
            previous = Some(current);
            ordinal = 0;
        }
        diagnostic.id = Some(format!("{path}:{start}-{end}:{code}:{report}:{ordinal}"));
        ordinal += 1;
    }
}

fn diagnostic_order(a: &Diagnostic, b: &Diagnostic) -> std::cmp::Ordering {
    (a.span.start.offset, &a.code, a.span.end.offset).cmp(&(
        b.span.start.offset,
        &b.code,
        b.span.end.offset,
    ))
}

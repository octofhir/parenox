mod extern_export;
mod navigation;
mod spec_proposals;
mod symbols;

pub use navigation::{
    ArchiveDeclarations, ArgumentKeyword, BindingType, CallHierarchyCalls, CallHierarchyItem,
    CallSignature, CallSignatures, Definition, DocumentHover, InspectedReference, LocalHover,
    PrivacyDeclaration, SemanticSymbol, SemanticSymbolKind, SourceExport, SourceHover,
    SourceTestKind, SourceTestNode, VarDocumentation, VarInfo, namespace_documentation,
    namespace_exports,
};
pub use spec_proposals::{ContractProposals, SpecContractProposal};
pub use symbols::{SourceSymbol, SourceSymbolKind};

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use serde::Serialize;

use crate::checker::{
    CheckedCall, Declarations, ExternCache, FunctionCheck, Globals, InterfaceKey, Model,
    ProjectCall, RuntimeLoads, SpecCache, SpecFingerprint, SpecSource,
};
use crate::reader::{self, Form, Position, Span};
use crate::suppressions::{MARKER, Suppressions};
use crate::{
    ContractOrigin, Diagnostic, ExternChangeKind, ExternDiff, ExternSnapshot, Report, Stats,
};

#[derive(Clone, Copy)]
pub struct Source<'a> {
    pub path: &'a str,
    pub source: &'a str,
}

/// An insertion at a UTF-8 byte offset in the current source snapshot.
pub struct SourceEdit {
    pub offset: usize,
    pub new_text: String,
}

/// A candidate contract. Drafts contain commented placeholders, not checked types.
pub struct FdefSuggestion {
    pub draft: bool,
    pub edits: Vec<SourceEdit>,
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
    navigation_fallback: Option<Box<NavigationFallback>>,
    hover_types: TypeQueries<String>,
    checked_calls: TypeQueries<Arc<CheckedCall>>,
    spec_fingerprint: Option<Arc<SpecFingerprint>>,
    model: Option<Model>,
    interface: Option<InterfaceKey>,
    checked_source: Arc<str>,
    functions: BTreeMap<String, CheckedFunction>,
    partial_diagnostics: Option<Vec<Diagnostic>>,
}

type TypeQueries<T> = OnceLock<Mutex<BTreeMap<usize, Option<T>>>>;

struct CheckedFunction {
    ranges: Vec<Span>,
    result: FunctionCheck,
}

struct NavigationFallback {
    forms: Vec<Form>,
    declarations: Declarations,
    checked_prefix: OnceLock<Option<navigation::CheckedPrefix>>,
    recovered_keywords: TypeQueries<Arc<[ArgumentKeyword]>>,
}

impl CheckedFunction {
    fn reusable(&self, previous_source: &str, source: &str, ranges: &[Span]) -> bool {
        // Diagnostics and call-chain notes contain absolute source positions.
        if !self.result.diagnostics.is_empty() && self.ranges != ranges {
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
enum SnapshotMode<'a> {
    Check(Option<&'a BTreeSet<&'a str>>),
    ValidateExterns,
    ExternSurface,
}

impl SnapshotMode<'_> {
    fn reports(self, path: &str) -> bool {
        match self {
            Self::Check(Some(paths)) => paths.contains(path),
            _ => true,
        }
    }
}

impl FileCache {
    fn update_runtime_loads(&mut self, loads: &RuntimeLoads) {
        let Some(declarations) = self
            .declarations
            .as_ref()
            .and_then(|declarations| declarations.with_runtime_loads(&self.forms, loads))
        else {
            return;
        };
        let fingerprint = declarations.spec_fingerprint(&self.source, &self.forms);
        if self.spec_fingerprint.as_deref() != fingerprint.as_ref() {
            self.spec_fingerprint = fingerprint.map(Arc::new);
        }
        self.declarations = Some(declarations);
        self.model = None;
        self.known_var_surface.take();
    }

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
        self.partial_diagnostics = None;
        self.parse_error = None;
        self.suppressions = Suppressions::default();
        self.declarations = None;
        self.known_var_surface.take();
        self.navigation_fallback = None;
        let conditionals = if kind == FileKind::Source && path.ends_with(".cljc") {
            reader::Conditionals::Select(":clj")
        } else {
            reader::Conditionals::Reject
        };
        let (mut read, read_error) = match kind {
            FileKind::Source => reader::read_for_check_recovering(
                source,
                conditionals,
                source.contains(MARKER).then_some(MARKER),
            ),
            FileKind::Externs => reader::read_for_check(source, conditionals, None),
        };
        let reader_failed = read_error.is_some();
        let parsed = match read_error {
            Some(error) => Err(error),
            None if kind == FileKind::Source => {
                Declarations::read_source(source, &mut read).map(Some)
            }
            None => match read.namespace_tokens.first() {
                Some(reader::NamespaceToken::Keyword(span)) => Err(reader::ReadError {
                    span: *span,
                    message: "Auto-resolved keywords are invalid in EDN externs".into(),
                }),
                Some(reader::NamespaceToken::Map { marker, .. }) => Err(reader::ReadError {
                    span: *marker,
                    message: "Auto-resolved maps are invalid in EDN externs".into(),
                }),
                None => Ok(None),
            },
        };
        match parsed {
            Ok(declarations) => {
                let fingerprint = declarations
                    .as_ref()
                    .and_then(|declarations| declarations.spec_fingerprint(source, &read.forms));
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
                    if !reader_failed {
                        // Preserve the valid prefix when namespace validation fails.
                        read.forms
                            .retain(|form| form.span.end.offset <= error.span.start.offset);
                    }
                    if !read.forms.is_empty() {
                        self.navigation_fallback = Some(Box::new(NavigationFallback {
                            declarations: Declarations::read(&read.forms),
                            forms: read.forms,
                            checked_prefix: OnceLock::new(),
                            recovered_keywords: OnceLock::new(),
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
            let recovered = self.navigation_fallback.as_deref()?;
            Some((&recovered.declarations, &recovered.forms))
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
    query_globals: Option<Globals>,
    snapshot: u64,
}

impl Session {
    /// Suggest an s/fdef at a top-level function in the current Clojure/JVM snapshot.
    pub fn suggest_fdef(&self, path: &str, offset: usize) -> Option<FdefSuggestion> {
        // In .cljc the selected reader branch need not be a top-level source form.
        if !path.ends_with(".clj") {
            return None;
        }
        let file = self.files.get(path).filter(|file| {
            file.snapshot == self.snapshot && file.source.is_char_boundary(offset)
        })?;
        file.declarations
            .as_ref()?
            .suggest_fdef(&file.forms, &file.source, offset)
    }

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
        self.check_snapshot(sources, externs, &[], SnapshotMode::Check(None))
    }

    /// Loads the complete snapshot for resolution and checks bodies for the selected paths.
    /// The report's status and diagnostics describe the selected files.
    pub fn check_project_documents(
        &mut self,
        sources: &[Source<'_>],
        externs: &[Source<'_>],
        paths: &BTreeSet<&str>,
    ) -> ProjectReport {
        self.check_snapshot(sources, externs, &[], SnapshotMode::Check(Some(paths)))
    }

    /// Check open documents and report changes to source calling types or their availability.
    pub fn check_project_documents_with_contract_changes(
        &mut self,
        sources: &[Source<'_>],
        externs: &[Source<'_>],
        paths: &BTreeSet<&str>,
    ) -> (ProjectReport, bool) {
        let previous_aliases: BTreeMap<_, _> = paths
            .iter()
            .filter_map(|&path| {
                let aliases = self.document_runtime_aliases(path);
                (!aliases.is_empty()).then_some((path, aliases))
            })
            .collect();
        let previous = self.query_globals.take();
        let report = self.check_project_documents(sources, externs, paths);
        let changed = previous
            .as_ref()
            .zip(self.query_globals.as_ref())
            .is_some_and(|(previous, current)| {
                !current.same_source_function_contracts(previous)
                    || paths.iter().any(|&path| {
                        let aliases = self.document_runtime_aliases(path);
                        previous_aliases
                            .get(path)
                            .map_or_else(|| !aliases.is_empty(), |old| old != &aliases)
                    })
            });
        (report, changed)
    }

    fn document_runtime_aliases(&self, path: &str) -> BTreeSet<String> {
        let Some(cache) = self
            .files
            .get(path)
            .filter(|cache| cache.snapshot == self.snapshot)
        else {
            return BTreeSet::new();
        };
        if let Some(declarations) = &cache.declarations {
            return declarations.proven_alias_libraries().clone();
        }
        cache
            .navigation_fallback
            .as_ref()
            .zip(self.query_globals.as_ref())
            .map_or_else(BTreeSet::new, |(fallback, globals)| {
                fallback
                    .declarations
                    .runtime_alias_evidence(&fallback.forms, &globals.runtime_loads)
            })
    }

    /// The declaration key for an active extern function in the latest snapshot.
    pub fn extern_declaration_span(&self, origin: &ContractOrigin) -> Option<Span> {
        self.externs.declaration_span(origin)
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
        mode: SnapshotMode<'_>,
    ) -> ProjectReport {
        let started = Instant::now();
        self.query_globals = None;
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
            // Type queries can depend on externs or other edited files.
            cache.hover_types.take();
            cache.checked_calls.take();
            if let Some(fallback) = &mut cache.navigation_fallback {
                fallback.checked_prefix.take();
                fallback.recovered_keywords.take();
            }
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
            entries.push((source, cache));
        }
        let mut namespaces = BTreeMap::new();
        let mut ambiguous = BTreeSet::new();
        for (index, (_, cache)) in entries[..count].iter().enumerate() {
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
        for (index, (_, cache)) in entries[..count].iter().enumerate() {
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
        let uses_runtime_aliases = entries[..count]
            .iter()
            .filter_map(|(_, cache)| cache.navigation_source())
            .any(|(declarations, _)| declarations.has_runtime_aliases());
        let runtime_loads = if uses_runtime_aliases {
            RuntimeLoads::from_sources(entries[..count].iter().enumerate().map(
                |(index, (source, cache))| {
                    let checked = built[index]
                        && cache.snapshot == self.snapshot
                        && cache.declarations.as_ref().is_some_and(|declarations| {
                            !ambiguous.contains(declarations.namespace())
                        });
                    (source.path, cache.navigation_source(), checked)
                },
            ))
        } else {
            RuntimeLoads::default()
        };
        if uses_runtime_aliases {
            for (_, cache) in &mut entries[..count] {
                cache.update_runtime_loads(&runtime_loads);
            }
        }
        self.specs.update(
            order
                .iter()
                .filter_map(|&index| {
                    let (source, cache) = &entries[index];
                    Some(SpecSource {
                        path: source.path,
                        source: source.source,
                        declarations: cache.declarations.as_ref()?,
                        fingerprint: cache.spec_fingerprint.as_ref()?,
                    })
                })
                .collect(),
            namespaces.contains_key("clojure.set"),
            namespaces.contains_key("clojure.core"),
        );
        let mut globals = Globals::with_specs(&self.specs, runtime_loads);
        for namespace in namespaces.keys() {
            globals.register_namespace(namespace);
        }
        for (source, cache) in &mut entries[count..] {
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
                .filter_map(|(_, cache)| cache.model.as_ref()?.extern_file()),
        );
        let trusted_paths = (mode == SnapshotMode::ValidateExterns && !dependencies.is_empty())
            .then(|| {
                let mut paths: BTreeSet<_> = dependencies.iter().map(|file| file.path).collect();
                // A path being validated cannot also provide trusted declarations.
                for file in externs {
                    paths.remove(file.path);
                }
                paths
            });
        if matches!(mode, SnapshotMode::Check(_)) {
            self.externs.publish_specs(&mut globals, None);
        } else if let Some(paths) = &trusted_paths {
            self.externs.publish_specs(&mut globals, Some(paths));
        }
        if mode != SnapshotMode::ExternSurface {
            for index in order {
                let (source, cache) = &mut entries[index];
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
        let source_literals =
            if mode == SnapshotMode::ValidateExterns && self.externs.has_literals() {
                entries[..count]
                    .iter()
                    .zip(&built)
                    .filter(|(_, built)| **built)
                    .flat_map(|((source, cache), _)| {
                        cache
                            .public_literals()
                            .map(move |(name, _, value)| (name.to_owned(), (source.path, value)))
                    })
                    .collect()
            } else {
                BTreeMap::new()
            };
        if matches!(mode, SnapshotMode::Check(_)) {
            self.externs.publish(&mut globals);
        } else if let Some(paths) = &trusted_paths {
            self.externs.publish_dependencies(paths, &mut globals);
        }
        let mut top_level_calls = BTreeMap::new();
        let mut checked_tests = BTreeMap::new();
        if matches!(mode, SnapshotMode::Check(_)) {
            let standard_test = !namespaces.contains_key("clojure.test");
            let standard_assertions = standard_test
                && !namespaces.contains_key("clojure.core")
                && !entries[..count].iter().any(|(_, cache)| {
                    cache
                        .declarations
                        .as_ref()
                        .is_some_and(|item| item.refers_to_assert_expr)
                });
            let provider = |namespace: &str| {
                let &index = namespaces.get(namespace)?;
                let (_, file) = &entries[index];
                let declarations = file.declarations.as_ref()?;
                (built[index] && file.snapshot == self.snapshot && !ambiguous.contains(namespace))
                    .then_some((file, declarations))
            };
            let known_provider = |namespace: &str| {
                provider(namespace).filter(|(file, declarations)| {
                    *file
                        .known_var_surface
                        .get_or_init(|| declarations.has_known_var_surface(&file.forms))
                })
            };
            for (index, (source, file)) in entries[..count].iter().enumerate() {
                if !mode.reports(source.path) {
                    continue;
                }
                if let (Some(declarations), Some(model)) = (&file.declarations, &file.model) {
                    let calls = declarations.top_level_call_diagnostics(
                        &file.forms,
                        |namespace, name| {
                            provider(namespace).is_some_and(|(_, declarations)| {
                                declarations.source_macro(name, None)
                            })
                        },
                        |namespace, name, actual| {
                            let Some((file, declarations)) = provider(namespace) else {
                                return ProjectCall::Unknown;
                            };
                            let call = declarations.project_call(&file.forms, name, actual);
                            // A matching declaration is evidence for a coverage gap, but a
                            // mismatch is only definite if the provider's var surface is known.
                            if matches!(
                                call,
                                ProjectCall::Function {
                                    expected_arity: Some(_)
                                }
                            ) && known_provider(namespace).is_none()
                            {
                                ProjectCall::Unknown
                            } else {
                                call
                            }
                        },
                    );
                    if !calls.is_empty() {
                        top_level_calls.insert(index, calls);
                    }
                    reports[index]
                        .diagnostics
                        .extend(declarations.check_referrals(|namespace, local| {
                            known_provider(namespace).is_some_and(|(provider, declarations)| {
                                declarations.missing_public_var(&provider.forms, local)
                            })
                        }));
                    reports[index]
                        .diagnostics
                        .extend(declarations.check_untyped_references(
                            &file.forms,
                            model,
                            standard_test,
                            |namespace, local, actual| {
                                known_provider(namespace).map_or(
                                    ProjectCall::Unknown,
                                    |(provider, declarations)| {
                                        declarations.project_call(&provider.forms, local, actual)
                                    },
                                )
                            },
                        ));
                    if standard_assertions {
                        let tests = declarations.check_test_bodies(
                            &file.forms,
                            model,
                            &globals,
                            source.source,
                            &mut reports[index].diagnostics,
                        );
                        if !tests.is_empty() {
                            checked_tests.insert(index, tests);
                        }
                    }
                }
            }
        }
        let core_in_extern_surface =
            mode == SnapshotMode::ExternSurface && namespaces.contains_key("clojure.core");
        let mut totals = Stats::default();
        let mut report_ordinals = BTreeMap::<&str, usize>::new();
        for (index, (source, mut cache)) in entries.into_iter().enumerate() {
            let file_started = Instant::now();
            let report = &mut reports[index];
            totals.parsed_files += report.stats.parsed_files;
            totals.reused_parses += report.stats.reused_parses;
            if !mode.reports(source.path) {
                self.files.insert(source.path.into(), cache);
                continue;
            }
            let specs_only = mode == SnapshotMode::ExternSurface && index < count;
            if specs_only {
                if let Some(declarations) = &cache.declarations {
                    report
                        .diagnostics
                        .extend_from_slice(declarations.diagnostics());
                    if core_in_extern_surface {
                        report
                            .diagnostics
                            .extend(declarations.reflection_flag_gaps());
                    }
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
                let calls = top_level_calls.get(&index);
                let tests = checked_tests.get(&index);
                let declaration_start = report.diagnostics.len();
                report
                    .diagnostics
                    .extend(model.diagnostics.iter().filter_map(|diagnostic| {
                        if diagnostic.code == "PNX9001" {
                            let offset = diagnostic.span.start.offset;
                            if tests.is_some_and(|tests| tests.contains(&offset)) {
                                return None;
                            }
                            if let Some(call) = calls.and_then(|calls| calls.get(&offset)) {
                                return Some(call.clone());
                            }
                        }
                        Some(diagnostic.clone())
                    }));
                let has_declaration_findings = report.diagnostics.len() != declaration_start;
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
                let interface = model.interface_key(&globals, has_declaration_findings);
                let changes = cache
                    .interface
                    .as_ref()
                    .map(|previous| interface.changes(previous));
                let same_specs = cache
                    .interface
                    .as_ref()
                    .is_some_and(|previous| interface.same_specs(previous));
                let unchanged = Arc::ptr_eq(&cache.checked_source, &cache.source)
                    && changes
                        .as_ref()
                        .is_some_and(|changes| changes.is_unchanged())
                    && (same_specs
                        || cache
                            .functions
                            .values()
                            .all(|cached| cached.result.matches_specs(&globals)));
                if !unchanged {
                    cache.partial_diagnostics = None;
                }
                if matches!(mode, SnapshotMode::Check(_))
                    && index < count
                    && let Some(declarations) = &cache.declarations
                {
                    report.diagnostics.extend(
                        cache
                            .partial_diagnostics
                            .get_or_insert_with(|| {
                                declarations.check_untyped_calls(
                                    &cache.forms,
                                    model,
                                    &globals,
                                    source.source,
                                )
                            })
                            .iter()
                            .cloned(),
                    );
                }
                if unchanged {
                    report.stats.reused_functions = cache.functions.len();
                    report.diagnostics.extend(
                        cache
                            .functions
                            .values()
                            .flat_map(|function| function.result.diagnostics.iter().cloned()),
                    );
                } else {
                    for function in model.functions.values().filter(|f| !f.inferred()) {
                        let ranges = model.function_ranges(function);
                        if let Some(cached) =
                            cache.functions.get_mut(&function.name).filter(|cached| {
                                cached.reusable(&cache.checked_source, source.source, &ranges)
                                    && (same_specs || cached.result.matches_specs(&globals))
                                    && changes.as_ref().is_some_and(|changes| {
                                        changes.allows_reuse(model, function, &cached.result)
                                    })
                            })
                        {
                            report.stats.reused_functions += 1;
                            report
                                .diagnostics
                                .extend(cached.result.diagnostics.iter().cloned());
                            cached.ranges = ranges;
                        } else {
                            report.stats.checked_functions += 1;
                            let result = model.check_function(
                                function,
                                source.source,
                                &globals,
                                has_declaration_findings,
                            );
                            report
                                .diagnostics
                                .extend(result.diagnostics.iter().cloned());
                            cache
                                .functions
                                .insert(function.name.clone(), CheckedFunction { ranges, result });
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
                model.diagnose_unchecked_helpers(
                    cache.functions.values().flat_map(|function| {
                        function.result.checked_helpers.iter().map(String::as_str)
                    }),
                    &mut report.diagnostics,
                );
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
            self.files.insert(source.path.into(), cache);
        }
        if matches!(mode, SnapshotMode::Check(Some(_))) {
            reports.retain(|report| mode.reports(&report.path));
        }
        if matches!(mode, SnapshotMode::Check(_)) {
            self.query_globals = Some(globals);
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

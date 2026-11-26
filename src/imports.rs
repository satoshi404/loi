//! Deterministic import-to-file resolution.
//!
//! The loader and the linker's `extern flow` collector both use this module, so
//! a path import and a logical import cannot disagree about which file they
//! name. Logical paths are anchored to the nearest `lib/` directory found by
//! walking ancestors of the *importing file*: no implicit cwd lookup, manifest,
//! download or generated table. Callers may add explicit library roots, checked
//! after those ancestors in a fixed order, so a compiler installation can ship
//! its own `lib/` without making resolution depend on the process cwd.
//! `src/modules.rs` remains the only resolver of qualified names and
//! visibility; this module only chooses source files and local module names.

use std::fs;
use std::path::{Path, PathBuf};

use crate::ast::{ImportDecl, ImportGroupItem, ImportKind, ImportName, Span};
use crate::diagnostic::Diagnostic;

/// One source file to load for an import declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedImport {
    pub path: PathBuf,
    /// Local module name bound by this import.
    pub alias: String,
    /// Span of the alias lexeme, or of the name it replaces.
    pub alias_span: Span,
    /// Span of the whole import declaration.
    pub declaration_span: Span,
    /// `open import` re-exports the imported module's public items.
    pub public: bool,
}

/// Failure to map an import declaration to a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    UnknownPackage {
        name: String,
        span: Span,
        searched: Vec<PathBuf>,
    },
    UnknownModule {
        path: String,
        span: Span,
        file: PathBuf,
    },
}

impl ResolveError {
    pub fn into_diagnostic(self) -> Diagnostic {
        match self {
            Self::UnknownPackage {
                name,
                span,
                searched,
            } => {
                let searched = searched
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                Diagnostic::from_error(format!("unknown package '{name}'"), span)
                    .with_label(format!(
                        "expected 'lib/{name}.loi' or 'lib/{name}/' near the importing file"
                    ))
                    .with_help(format!(
                        "logical imports resolve from the nearest lib/ directory next to the importing file; searched: {searched}"
                    ))
            }
            Self::UnknownModule { path, span, file } => {
                Diagnostic::from_error(format!("unknown module '{path}'"), span)
                    .with_label(format!("expected file '{}'", file.display()))
                    .with_help("add the module file or use an explicit path import")
            }
        }
    }
}

/// Map one import declaration to every source file it loads.
///
/// A path import loads one file. A logical import loads either a package root
/// (`import std` → `lib/std.loi`) or one narrow module file; a grouped import
/// loads one file per item. Aliases are explicit overrides of the last path
/// segment.
pub fn resolve_import(
    import: &ImportDecl,
    from: &Path,
) -> Result<Vec<ResolvedImport>, ResolveError> {
    resolve_import_with(import, from, &[])
}

/// [`resolve_import`] with extra library roots checked after the ancestors of
/// `from`. Roots are directories that contain `<package>.loi` or `<package>/`,
/// and are tried in the order given. Project-local `lib/` directories always
/// win, so adding a root can never shadow declarations already visible to the
/// importing file. Nothing is searched implicitly.
pub fn resolve_import_with(
    import: &ImportDecl,
    from: &Path,
    extra_roots: &[PathBuf],
) -> Result<Vec<ResolvedImport>, ResolveError> {
    match &import.kind {
        ImportKind::Path { span, path, alias } => {
            let file = resolve_path(from, path);
            let (alias, alias_span) = match alias {
                Some(name) => (name.name.clone(), name.span),
                None => (module_name(&file), *span),
            };
            Ok(vec![resolved(import, file, alias, alias_span)])
        }
        ImportKind::Logical { segments, alias } => {
            let root = find_library_root(from, &segments[0], extra_roots)?;
            let (file, path, span) = if segments.len() == 1 {
                let package = &segments[0];
                (
                    root.join(format!("{}.loi", package.name)),
                    package.name.clone(),
                    package.span,
                )
            } else {
                let file = logical_file(&root, segments);
                (file, qualified(segments), segments.last().unwrap().span)
            };
            require_file(&file, path, span)?;
            let (alias, alias_span) = match alias {
                Some(name) => (name.name.clone(), name.span),
                None if segments.len() == 1 => {
                    let package = &segments[0];
                    (package.name.clone(), package.span)
                }
                None => {
                    let last = segments.last().unwrap();
                    (last.name.clone(), last.span)
                }
            };
            Ok(vec![resolved(import, file, alias, alias_span)])
        }
        ImportKind::Group { base, items } => {
            let root = find_library_root(from, &base[0], extra_roots)?;
            let mut resolved_imports = Vec::with_capacity(items.len());
            for item in items {
                let file = group_item_file(&root, base, item);
                require_file(&file, group_item_path(base, item), item.name.span)?;
                let (alias, alias_span) = match &item.alias {
                    Some(name) => (name.name.clone(), name.span),
                    None => (item.name.name.clone(), item.name.span),
                };
                resolved_imports.push(resolved(import, file, alias, alias_span));
            }
            Ok(resolved_imports)
        }
    }
}

fn resolved(import: &ImportDecl, path: PathBuf, alias: String, alias_span: Span) -> ResolvedImport {
    ResolvedImport {
        path,
        alias,
        alias_span,
        declaration_span: import.span,
        public: import.public,
    }
}

fn resolve_path(from: &Path, requested: &str) -> PathBuf {
    let path = Path::new(requested);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        // Keep the importing file's spelling for path imports so existing
        // diagnostics still point at `lib/os/time.loi`, not an absolute path.
        from.parent().unwrap_or_else(|| Path::new(".")).join(path)
    }
}

/// The nearest `lib/<package>` layout containing a source file.
///
/// This is the filesystem counterpart of logical import resolution: native
/// dependencies are discovered from the same package layout the loader uses,
/// never from the process cwd or an unrelated ancestor directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageLayout {
    /// The `lib` directory, e.g. `/repo/lib`.
    pub root: PathBuf,
    /// The package directory immediately below `root`, e.g. `os`.
    pub name: String,
}

/// Find the `lib/<package>` layout containing `path`, if any.
pub fn package_layout(path: &Path) -> Option<PackageLayout> {
    let absolute = absolute_path(path);
    let mut dir = absolute.parent()?;
    loop {
        let lib = dir.join("lib");
        if let Ok(rest) = absolute.strip_prefix(&lib) {
            let mut components = rest.components();
            let package = components.next()?;
            // A file directly inside `lib/` is a flat module, not a package.
            if components.next().is_some()
                && let Some(name) = package.as_os_str().to_str()
            {
                return Some(PackageLayout {
                    root: lib,
                    name: name.to_string(),
                });
            }
        }
        let parent = dir.parent()?;
        if parent == dir {
            return None;
        }
        dir = parent;
    }
}

/// C shim files that may provide an `extern flow` declared in `path`.
///
/// The shim beside the declaring module wins; a package-root shim is the
/// fallback. Candidates outside the nearest `lib/<package>` layout are not
/// considered, so selecting a target can never depend on unrelated files.
pub fn shim_candidates(path: &Path) -> Vec<PathBuf> {
    shim_candidates_with(path, &[])
}

/// [`shim_candidates`] with explicit library roots, mirroring
/// [`resolve_import_with`]. A declaring file under an extra root contributes
/// that root's `<package>_shim.c` even when the directory is not named `lib`.
pub fn shim_candidates_with(path: &Path, extra_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut push = |candidate: PathBuf| {
        let key = fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
        let duplicate = candidates.iter().any(|existing: &PathBuf| {
            fs::canonicalize(existing).unwrap_or_else(|_| existing.clone()) == key
        });
        if !duplicate {
            candidates.push(candidate);
        }
    };
    if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
        push(path.with_file_name(format!("{stem}_shim.c")));
    }
    if let Some(layout) = package_layout(path) {
        push(layout.root.join(format!("{}_shim.c", layout.name)));
    }
    let absolute = absolute_path(path);
    for root in extra_roots {
        let Ok(rest) = absolute.strip_prefix(absolute_path(root)) else {
            continue;
        };
        let mut components = rest.components();
        let Some(package) = components.next() else {
            continue;
        };
        // A file directly inside the root is a flat module, not a package.
        if components.next().is_none() {
            continue;
        }
        if let Some(name) = package.as_os_str().to_str() {
            push(root.join(format!("{name}_shim.c")));
        }
    }
    candidates
}

fn absolute_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        }
    })
}

/// Absolute directory containing the importing file, independent of cwd.
fn absolute_dir(from: &Path) -> PathBuf {
    let absolute = absolute_path(from);
    absolute.parent().unwrap_or(&absolute).to_path_buf()
}

fn find_library_root(
    from: &Path,
    package: &ImportName,
    extra_roots: &[PathBuf],
) -> Result<PathBuf, ResolveError> {
    let mut searched = Vec::new();
    let mut dir = absolute_dir(from);
    loop {
        let lib = dir.join("lib");
        searched.push(lib.clone());
        if has_package(&lib, &package.name) {
            return Ok(lib);
        }
        let Some(parent) = dir.parent() else {
            break;
        };
        if parent == dir {
            break;
        }
        dir = parent.to_path_buf();
    }
    for root in extra_roots {
        if !searched.contains(root) {
            searched.push(root.clone());
        }
        if has_package(root, &package.name) {
            return Ok(root.clone());
        }
    }
    Err(ResolveError::UnknownPackage {
        name: package.name.clone(),
        span: package.span,
        searched,
    })
}

fn has_package(root: &Path, package: &str) -> bool {
    root.join(format!("{package}.loi")).is_file() || root.join(package).is_dir()
}

fn logical_file(root: &Path, segments: &[ImportName]) -> PathBuf {
    let mut file = root.join(&segments[0].name);
    let last = segments.len() - 1;
    for segment in &segments[1..last] {
        file.push(&segment.name);
    }
    file.push(format!("{}.loi", segments[last].name));
    file
}

fn group_item_file(root: &Path, base: &[ImportName], item: &ImportGroupItem) -> PathBuf {
    let mut file = root.join(&base[0].name);
    for segment in &base[1..] {
        file.push(&segment.name);
    }
    file.push(format!("{}.loi", item.name.name));
    file
}

fn qualified(segments: &[ImportName]) -> String {
    segments
        .iter()
        .map(|segment| segment.name.as_str())
        .collect::<Vec<_>>()
        .join("::")
}

fn group_item_path(base: &[ImportName], item: &ImportGroupItem) -> String {
    format!("{}::{}", qualified(base), item.name.name)
}

fn require_file(file: &Path, name: String, span: Span) -> Result<(), ResolveError> {
    if file.is_file() {
        Ok(())
    } else {
        Err(ResolveError::UnknownModule {
            path: name,
            span,
            file: file.to_path_buf(),
        })
    }
}

fn module_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("module")
        .to_string()
}

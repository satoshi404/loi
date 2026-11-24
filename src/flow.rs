//! Internal flow-contract primitives shared by the checker analyses.
//!
//! These types are compiler-only. They do not introduce new syntax and are not
//! emitted to either backend. The empty `Vec<PathSegment>` means the whole
//! root value, matching the historical `""` path string used by the checker.
//! Field segments render without a leading dot at the start of a relative path
//! because the checker's `join_paths` supplies separators; subsequent fields
//! keep the checker's `.field` spelling.

use std::collections::{BTreeMap, BTreeSet};

/// A constant index keeps negative literals for compatibility with the
/// checker's existing `i128` path parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IndexKey {
    Const(i128),
    /// A runtime index. It may alias any constant or other dynamic index.
    Dynamic,
    /// An index whose provenance is unknown; treated like `Dynamic` but kept
    /// distinct in diagnostics.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PathSegment {
    Field(String),
    Index(IndexKey),
    /// Opaque suffix. It overlaps every place below it.
    UnknownTail,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct DataPath {
    pub root: String,
    pub segments: Vec<PathSegment>,
}

impl DataPath {
    pub fn new(root: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            segments: Vec::new(),
        }
    }

    pub fn field(mut self, name: impl Into<String>) -> Self {
        self.segments
            .push(PathSegment::Field(name.into().replace('.', "")));
        self
    }

    pub fn index(mut self, key: IndexKey) -> Self {
        self.segments.push(PathSegment::Index(key));
        self
    }

    pub fn unknown_tail(mut self) -> Self {
        self.segments.push(PathSegment::UnknownTail);
        self
    }

    pub fn is_whole(&self) -> bool {
        self.segments.is_empty()
    }

    /// Parse a checker-style root-relative string (`field`, `a.b`, `[0]`,
    /// `[]`). An optional leading dot is accepted for readability.
    pub fn from_relative(root: impl Into<String>, relative: &str) -> Self {
        let mut path = Self::new(root);
        let chars: Vec<char> = relative.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '.' {
                i += 1;
                continue;
            }
            if chars[i] == '[' {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != ']' {
                    i += 1;
                }
                let raw: String = chars[start..i].iter().collect();
                if i < chars.len() {
                    i += 1;
                }
                match raw.parse::<i128>() {
                    Ok(index) => path = path.index(IndexKey::Const(index)),
                    Err(_) => path = path.index(IndexKey::Dynamic),
                }
                continue;
            }
            let start = i;
            while i < chars.len() && chars[i] != '.' && chars[i] != '[' {
                i += 1;
            }
            let field: String = chars[start..i].iter().collect();
            if !field.is_empty() {
                path = path.field(field);
            }
        }
        path
    }

    /// Render only the root-relative part, using the checker's historical
    /// spelling. The first field has no leading dot; later fields and indexes
    /// are appended verbatim.
    pub fn relative_string(&self) -> String {
        let mut rendered = String::new();
        for segment in &self.segments {
            match segment {
                PathSegment::Field(field) => {
                    if !rendered.is_empty() {
                        rendered.push('.');
                    }
                    rendered.push_str(field);
                }
                PathSegment::Index(IndexKey::Const(index)) => {
                    rendered.push('[');
                    rendered.push_str(&index.to_string());
                    rendered.push(']');
                }
                PathSegment::Index(IndexKey::Dynamic | IndexKey::Unknown) => {
                    rendered.push_str("[]");
                }
                PathSegment::UnknownTail => rendered.push_str("[]"),
            }
        }
        rendered
    }

    fn segment_prefix_matches(prefix: &PathSegment, other: &PathSegment) -> bool {
        match (prefix, other) {
            (PathSegment::Field(left), PathSegment::Field(right)) => left == right,
            (
                PathSegment::Index(IndexKey::Const(left)),
                PathSegment::Index(IndexKey::Const(right)),
            ) => left == right,
            (PathSegment::Index(IndexKey::Dynamic | IndexKey::Unknown), PathSegment::Index(_)) => {
                true
            }
            (PathSegment::Index(_), PathSegment::Index(IndexKey::Dynamic | IndexKey::Unknown)) => {
                true
            }
            (PathSegment::UnknownTail, _) | (_, PathSegment::UnknownTail) => true,
            _ => false,
        }
    }

    /// Whether `self` is a structural prefix of `other`. A dynamic index
    /// prefix contains every constant element, matching the checker's current
    /// conservative rule.
    pub fn is_ancestor_of(&self, other: &Self) -> bool {
        self.root == other.root
            && self.segments.len() <= other.segments.len()
            && self
                .segments
                .iter()
                .zip(other.segments.iter())
                .all(|(left, right)| Self::segment_prefix_matches(left, right))
    }

    pub fn may_overlap(&self, other: &Self) -> bool {
        if self.root != other.root {
            return false;
        }
        let len = self.segments.len().min(other.segments.len());
        self.segments[..len]
            .iter()
            .zip(other.segments[..len].iter())
            .all(|(left, right)| Self::segment_prefix_matches(left, right))
    }

    /// Strong alias proof: only exact fields and constant indexes qualify.
    /// Dynamic indexes and opaque tails are never considered proven equal.
    pub fn definitely_same(&self, other: &Self) -> bool {
        self.root == other.root
            && self.segments == other.segments
            && !self.segments.iter().any(|segment| {
                matches!(
                    segment,
                    PathSegment::Index(IndexKey::Dynamic | IndexKey::Unknown)
                        | PathSegment::UnknownTail
                )
            })
    }
}

/// Deterministic set of paths with ancestor collapsing. Inserting a whole
/// value subsumes every narrower path, matching `insert_consumed_path`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataPathSet {
    paths: BTreeSet<DataPath>,
}

impl DataPathSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, path: DataPath) -> bool {
        if self
            .paths
            .iter()
            .any(|existing| existing.is_ancestor_of(&path))
        {
            return false;
        }
        if path.is_whole() {
            self.paths.clear();
            self.paths.insert(path);
            return true;
        }
        self.paths.retain(|existing| !path.is_ancestor_of(existing));
        self.paths.insert(path)
    }

    pub fn contains_overlapping(&self, path: &DataPath) -> bool {
        self.paths.iter().any(|existing| existing.may_overlap(path))
    }

    pub fn overlaps_effect(&self, effect: &FlowEffect) -> bool {
        effect
            .paths()
            .iter()
            .any(|path| self.contains_overlapping(path))
    }

    pub fn iter(&self) -> impl Iterator<Item = &DataPath> {
        self.paths.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn relative_strings(&self) -> Vec<String> {
        self.paths.iter().map(DataPath::relative_string).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BorrowMode {
    Shared,
    Exclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum LoanSource {
    Param { index: usize, path: DataPath },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum LoanDestination {
    Result,
    OutParam(usize),
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Borrow {
    pub source: LoanSource,
    pub holder: DataPath,
    pub destination: LoanDestination,
    pub mode: BorrowMode,
    pub escaped: bool,
}

impl Borrow {
    /// The borrowed place. The holder is where the view is stored, not what the
    /// loan touches, so it is deliberately excluded from source-path overlap.
    pub fn source_path(&self) -> Option<&DataPath> {
        match &self.source {
            LoanSource::Param { path, .. } => Some(path),
            LoanSource::Unknown => None,
        }
    }

    pub fn source_param(&self) -> Option<usize> {
        match self.source {
            LoanSource::Param { index, .. } => Some(index),
            LoanSource::Unknown => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowEffect {
    Read(DataPath),
    Write(DataPath),
    Init(DataPath),
    Move(DataPath),
    Borrow(Borrow),
    Produce(DataPath),
    Escape(DataPath),
}

impl FlowEffect {
    /// Places touched by this effect. A loan touches its source place; the
    /// holder is a separate destination and is compared by `ResultEffects`.
    pub fn paths(&self) -> Vec<&DataPath> {
        match self {
            Self::Read(path)
            | Self::Write(path)
            | Self::Init(path)
            | Self::Move(path)
            | Self::Produce(path)
            | Self::Escape(path) => vec![path],
            Self::Borrow(borrow) => borrow.source_path().into_iter().collect(),
        }
    }

    pub fn overlaps_path(&self, path: &DataPath) -> bool {
        self.paths()
            .iter()
            .any(|candidate| candidate.may_overlap(path))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParamEffects {
    pub reads: DataPathSet,
    pub writes: DataPathSet,
    pub inits: DataPathSet,
    pub moves: DataPathSet,
    pub borrows: Vec<Borrow>,
    pub escapes: DataPathSet,
}

impl ParamEffects {
    pub fn record(&mut self, effect: FlowEffect) -> bool {
        match effect {
            FlowEffect::Read(path) => self.reads.insert(path),
            FlowEffect::Write(path) => self.writes.insert(path),
            FlowEffect::Init(path) => self.inits.insert(path),
            FlowEffect::Move(path) => self.moves.insert(path),
            FlowEffect::Borrow(borrow) => {
                self.borrows.push(borrow);
                self.borrows.sort();
                self.borrows.dedup();
                true
            }
            FlowEffect::Escape(path) => self.escapes.insert(path),
            FlowEffect::Produce(_) => false,
        }
    }

    /// Whether any recorded param effect touches the same place as `effect`.
    /// All effects share the `DataPath::may_overlap` rule, including loans.
    pub fn overlaps(&self, effect: &FlowEffect) -> bool {
        let paths = effect.paths();
        if paths.is_empty() {
            return false;
        }
        let touches = |set: &DataPathSet| paths.iter().any(|path| set.contains_overlapping(path));
        touches(&self.reads)
            || touches(&self.writes)
            || touches(&self.inits)
            || touches(&self.moves)
            || touches(&self.escapes)
            || self.borrows.iter().any(|borrow| {
                borrow
                    .source_path()
                    .is_some_and(|source| paths.iter().any(|path| source.may_overlap(path)))
            })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultEffects {
    pub produces: bool,
    pub paths: DataPathSet,
    pub regions: BTreeSet<usize>,
    pub loans: Vec<Borrow>,
}

impl ResultEffects {
    /// Whether the result described by this summary touches `effect`.
    /// Producing the whole result overlaps every produced subpath; loans are
    /// compared by both source and holder so result slots and borrowed places
    /// use one path-overlap rule.
    pub fn overlaps(&self, effect: &FlowEffect) -> bool {
        let paths = effect.paths();
        if paths.is_empty() {
            return false;
        }
        let touches = |path: &DataPath| paths.iter().any(|candidate| candidate.may_overlap(path));
        match effect {
            FlowEffect::Produce(_) => {
                self.produces && (self.paths.is_empty() || self.paths.iter().any(&touches))
            }
            FlowEffect::Borrow(_) => {
                self.paths.iter().any(&touches)
                    || self.loans.iter().any(|loan| {
                        loan.source_path().is_some_and(&touches) || touches(&loan.holder)
                    })
            }
            _ => {
                self.paths.iter().any(&touches)
                    || self.loans.iter().any(|loan| touches(&loan.holder))
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowSummary {
    pub params: Vec<ParamEffects>,
    pub result: ResultEffects,
    pub unknown_effects: bool,
    pub allocations: bool,
    /// Allocation reachable outside every `protect`, for compatibility with
    /// `unguarded_allocators`.
    pub unguarded_allocations: bool,
    /// Allocation caused directly or transitively by `clone`.
    pub clone_allocations: bool,
}

impl FlowSummary {
    pub fn with_params(count: usize) -> Self {
        Self {
            params: vec![ParamEffects::default(); count],
            ..Self::default()
        }
    }

    pub fn ensure_params(&mut self, count: usize) {
        if self.params.len() < count {
            self.params.resize_with(count, ParamEffects::default);
        }
    }

    pub fn record_param_effect(&mut self, index: usize, effect: FlowEffect) -> bool {
        self.ensure_params(index + 1);
        self.params[index].record(effect)
    }

    /// Whether `effect` touches a parameter, including loans that escape into
    /// the result. Result loans are filtered by source index so one caller
    /// argument is not confused with another.
    pub fn param_effects_overlap(&self, index: usize, effect: &FlowEffect) -> bool {
        if self
            .params
            .get(index)
            .is_some_and(|param| param.overlaps(effect))
        {
            return true;
        }
        self.result.loans.iter().any(|loan| {
            loan.source_param() == Some(index)
                && loan
                    .source_path()
                    .is_some_and(|source| effect.overlaps_path(source))
        })
    }

    /// Legacy accessor for `param_consumption`.
    pub fn consumed_paths(&self, index: usize) -> Vec<String> {
        self.params
            .get(index)
            .map(|param| param.moves.relative_strings())
            .unwrap_or_default()
    }

    /// Legacy accessor for `param_reads`.
    pub fn read_paths(&self, index: usize) -> Vec<String> {
        self.params
            .get(index)
            .map(|param| param.reads.relative_strings())
            .unwrap_or_default()
    }

    /// Legacy accessor for `view_returns`.
    pub fn view_returns(&self) -> BTreeSet<usize> {
        self.borrows()
            .filter(|borrow| {
                borrow.escaped && matches!(borrow.destination, LoanDestination::Result)
            })
            .filter_map(|borrow| match borrow.source {
                LoanSource::Param { index, .. } => Some(index),
                LoanSource::Unknown => None,
            })
            .collect()
    }

    /// Legacy accessor for `view_out`.
    pub fn view_out(&self) -> BTreeMap<usize, BTreeSet<usize>> {
        let mut out: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for borrow in self.borrows() {
            if !borrow.escaped {
                continue;
            }
            let LoanDestination::OutParam(destination) = borrow.destination else {
                continue;
            };
            if let LoanSource::Param { index: source, .. } = borrow.source {
                out.entry(destination).or_default().insert(source);
            }
        }
        out
    }

    fn borrows(&self) -> impl Iterator<Item = &Borrow> {
        self.params
            .iter()
            .flat_map(|param| param.borrows.iter())
            .chain(self.result.loans.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_strings_round_trip() {
        let path = DataPath::from_relative("p", "a[0].b[]");
        assert_eq!(path.relative_string(), "a[0].b[]");
    }

    #[test]
    fn unknown_tail_overlaps_every_extension() {
        let opaque = DataPath::new("p").unknown_tail();
        assert!(opaque.may_overlap(&DataPath::new("p").field("anything")));
        assert!(!opaque.definitely_same(&DataPath::new("p").field("anything")));
    }
}

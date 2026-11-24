use loi::flow::{
    Borrow, BorrowMode, DataPath, DataPathSet, FlowEffect, FlowSummary, IndexKey, LoanDestination,
    LoanSource,
};
#[test]
fn independent_fields_are_disjoint() {
    let physics = DataPath::new("world").field("physics");
    let graphics = DataPath::new("world").field("graphics");
    assert!(!physics.may_overlap(&graphics));
}

#[test]
fn nested_paths_overlap_and_ancestor_is_recognised() {
    let whole = DataPath::new("world").field("physics");
    let nested = whole.clone().field("velocity").field("x");
    assert!(whole.may_overlap(&nested));
    assert!(whole.is_ancestor_of(&nested));
    assert!(!nested.is_ancestor_of(&whole));
}

#[test]
fn constant_indices_are_disjoint_but_dynamic_indices_are_not() {
    let left = DataPath::new("xs").index(IndexKey::Const(0));
    let right = DataPath::new("xs").index(IndexKey::Const(1));
    let dynamic = DataPath::new("xs").index(IndexKey::Dynamic);
    assert!(!left.may_overlap(&right));
    assert!(left.may_overlap(&dynamic));
    assert!(dynamic.may_overlap(&DataPath::new("xs").index(IndexKey::Dynamic)));
}

#[test]
fn path_set_collapses_ancestors_and_whole_values() {
    let mut paths = DataPathSet::new();
    paths.insert(DataPath::new("p").field("left"));
    paths.insert(DataPath::new("p").field("left").field("x"));
    assert_eq!(paths.relative_strings(), vec!["left"]);

    paths.insert(DataPath::new("p"));
    assert_eq!(paths.relative_strings(), vec![""]);
}

#[test]
fn flow_summary_exposes_legacy_consumption_and_read_paths() {
    let mut summary = FlowSummary::with_params(2);
    summary.params[0]
        .moves
        .insert(DataPath::from_relative("p", "left"));
    summary.params[1]
        .reads
        .insert(DataPath::from_relative("xs", "len"));

    assert_eq!(summary.consumed_paths(0), vec!["left"]);
    assert_eq!(summary.read_paths(1), vec!["len"]);
}

#[test]
fn escaped_borrows_derive_view_summaries() {
    let mut summary = FlowSummary::with_params(2);
    summary.params[0].borrows.push(Borrow {
        source: LoanSource::Param {
            index: 0,
            path: DataPath::new("data"),
        },
        holder: DataPath::new("part"),
        destination: LoanDestination::Result,
        mode: BorrowMode::Shared,
        escaped: true,
    });
    summary.params[1].borrows.push(Borrow {
        source: LoanSource::Param {
            index: 0,
            path: DataPath::new("data"),
        },
        holder: DataPath::new("out"),
        destination: LoanDestination::OutParam(1),
        mode: BorrowMode::Shared,
        escaped: true,
    });

    assert!(summary.view_returns().contains(&0));
    assert!(
        summary
            .view_out()
            .get(&1)
            .is_some_and(|sources| sources.contains(&0))
    );
}

#[test]
fn every_effect_kind_uses_the_same_path_overlap_rule() {
    let mut summary = FlowSummary::with_params(1);
    let param = &mut summary.params[0];
    assert!(param.reads.insert(DataPath::from_relative("p", "left")));
    assert!(param.writes.insert(DataPath::from_relative("p", "right")));
    assert!(param.inits.insert(DataPath::from_relative("p", "middle")));
    assert!(param.moves.insert(DataPath::from_relative("p", "left.x")));
    assert!(
        param
            .escapes
            .insert(DataPath::from_relative("p", "right[0]"))
    );
    param.borrows.push(Borrow {
        source: LoanSource::Param {
            index: 0,
            path: DataPath::from_relative("p", "middle"),
        },
        holder: DataPath::new("held"),
        destination: LoanDestination::Local,
        mode: BorrowMode::Shared,
        escaped: false,
    });

    assert!(param.overlaps(&FlowEffect::Read(DataPath::from_relative("p", "left"))));
    assert!(param.overlaps(&FlowEffect::Write(DataPath::from_relative("p", "right[0]"))));
    assert!(param.overlaps(&FlowEffect::Init(DataPath::from_relative("p", "middle"))));
    assert!(param.overlaps(&FlowEffect::Move(DataPath::from_relative("p", "left"))));
    assert!(param.overlaps(&FlowEffect::Escape(DataPath::from_relative("p", "right"))));
    assert!(param.overlaps(&FlowEffect::Borrow(Borrow {
        source: LoanSource::Param {
            index: 0,
            path: DataPath::from_relative("p", "left.x"),
        },
        holder: DataPath::new("other"),
        destination: LoanDestination::Result,
        mode: BorrowMode::Shared,
        escaped: true,
    })));
    assert!(!param.overlaps(&FlowEffect::Read(DataPath::new("p").field("other"))));
}

#[test]
fn result_effects_overlap_produce_and_loans() {
    let mut summary = FlowSummary::with_params(1);
    summary.result.produces = true;
    summary
        .result
        .paths
        .insert(DataPath::new("result").field("field"));
    summary.result.loans.push(Borrow {
        source: LoanSource::Param {
            index: 0,
            path: DataPath::new("xs"),
        },
        holder: DataPath::new("result"),
        destination: LoanDestination::Result,
        mode: BorrowMode::Shared,
        escaped: true,
    });

    assert!(
        summary
            .result
            .overlaps(&FlowEffect::Produce(DataPath::new("result").field("field")))
    );
    assert!(summary.param_effects_overlap(
        0,
        &FlowEffect::Borrow(Borrow {
            source: LoanSource::Param {
                index: 0,
                path: DataPath::new("xs"),
            },
            holder: DataPath::new("result"),
            destination: LoanDestination::Result,
            mode: BorrowMode::Shared,
            escaped: true,
        })
    ));
}

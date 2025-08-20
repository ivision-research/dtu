use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::iter::Iterator;

use graph::prelude::DirectedNeighborsWithValues;
use miette::{bail, IntoDiagnostic, Result};
use rayon::prelude::*;
use smartstring::{LazyCompact, SmartString};

use cozo::{
    DataValue, Expr, FixedRule, FixedRulePayload, Poison, RegularTempStore, SourceSpan, Symbol,
};

/// Find all paths out a given set of nodes
///
/// This is a pretty slow operation and the time it takes to run grows very fast with depth.
pub(crate) struct FindReachableDFS;

impl FixedRule for FindReachableDFS {
    fn run(
        &self,
        payload: FixedRulePayload<'_, '_>,
        out: &mut RegularTempStore,
        poison: Poison,
    ) -> Result<()> {
        let edges = payload.get_input(0)?;
        // The methods we start from
        let start_methods = payload.get_input(1)?;

        // The max depth we want to go, we set this to 2 by default.
        let mut max_depth = payload.pos_integer_option("depth", Some(2))?;

        if max_depth > 5 {
            log::warn!("Large depth detected for call search: may take a long time!");
        } else if max_depth == 0 {
            log::warn!("Invalid 0 max depth, using 1");
            max_depth = 1;
        }

        let (graph, indices, inv_indices) = edges.as_directed_graph(false)?;

        let mut sources = BTreeSet::new();
        for tuple in start_methods.iter()? {
            let tuple = tuple?;
            let node = &tuple[0];
            if let Some(idx) = inv_indices.get(node) {
                sources.insert(*idx);
            }
        }

        let do_search = |start: u32| -> Result<Vec<Vec<u32>>> {
            let mut seen = BTreeSet::new();
            let mut stack: Vec<(u32, usize)> = Vec::new();
            let mut paths: Vec<Vec<u32>> = Vec::new();
            let mut backpointers: Vec<u32> = vec![0u32; max_depth + 1];
            stack.push((start, 0));

            while let Some((candidate, depth)) = stack.pop() {
                // A real recusion check, if the candidate is already in backpointers up to
                // this depth we've hit a cycle. I think this should be cheaper in general than
                // going down that path?

                let current_path = &backpointers[0..depth];
                if current_path.contains(&candidate) {
                    continue;
                }

                backpointers[depth] = candidate;
                let depth = depth + 1;

                seen.clear();

                let mut added = false;

                for target in graph.out_neighbors_with_values(candidate) {
                    let next_node = target.target;

                    // next_node == candidate is a simple recursion check, that
                    // won't catch all cycles, but should help here.
                    if next_node == candidate || !seen.insert(next_node) {
                        continue;
                    }

                    if depth < max_depth {
                        added = true;
                        stack.push((next_node, depth));
                    } else {
                        backpointers[depth] = next_node;
                        let backslice = backpointers.as_slice();
                        let path = &backslice[0..depth + 1];
                        paths.push(path.into());
                    }

                    poison.check()?;
                }

                // If we stopped going down this path, add what we've got
                if !added {
                    let backslice = backpointers.as_slice();
                    let path = &backslice[0..depth];
                    paths.push(path.into());
                }
            }
            Ok(paths)
        };

        let paths = if sources.len() == 1 {
            if let Some(start) = sources.first() {
                do_search(*start)?
            } else {
                Vec::new()
            }
        } else {
            let it = sources.into_par_iter();

            let res = it
                .map(do_search)
                .filter_map(|it| it.ok())
                .collect::<Vec<Vec<Vec<u32>>>>();

            poison.check()?;

            res.into_iter().flatten().collect::<Vec<Vec<u32>>>()
        };

        for path in paths {
            let t = vec![
                indices[*path.first().unwrap() as usize].clone(),
                indices[*path.last().unwrap() as usize].clone(),
                DataValue::List(
                    path.into_iter()
                        .map(|u| indices[u as usize].clone())
                        .collect::<Vec<DataValue>>(),
                ),
            ];
            out.put(t);
        }

        Ok(())
    }

    fn arity(
        &self,
        _options: &BTreeMap<SmartString<LazyCompact>, Expr>,
        _rule_head: &[Symbol],
        _span: SourceSpan,
    ) -> Result<usize> {
        Ok(3)
    }
}

/// Find all paths between two nodes
///
/// This is a pretty slow operation and the time it takes to run grows very fast with depth. We use
/// this to find calls to a given method, but it will always be incomplete because of the depth.
/// Note that it is also very important for performance to define as small as possible of a start
/// set.
pub(crate) struct FindPathsDFS;

impl FixedRule for FindPathsDFS {
    fn run(
        &self,
        payload: FixedRulePayload<'_, '_>,
        out: &mut RegularTempStore,
        poison: Poison,
    ) -> Result<()> {
        #[derive(PartialEq, Eq, Ord, PartialOrd, Default, Clone)]
        struct ClassSource {
            class: u32,
            source: String,
        }

        // The whole graph
        let edges = payload.get_input(0)?;
        edges.ensure_min_len(2)?;

        // The methods we start from, including their source
        let start_methods = payload.get_input(1)?;
        start_methods.ensure_min_len(2)?;

        // The methods we're looking to find calls into
        let target_methods = payload.get_input(2)?;

        // The max depth we want to go, we set this to 2 by default.
        let mut max_depth = payload.pos_integer_option("depth", Some(2))?;

        if max_depth > 5 {
            log::warn!("Large depth detected for call search: may take a long time!");
        } else if max_depth == 0 {
            log::warn!("Invalid 0 max depth, using 1");
            max_depth = 1;
        }

        let (graph, indices, inv_indices) = edges.as_directed_graph(false)?;

        // Just grab indices for the targets
        let mut targets = BTreeSet::new();
        for tuple in target_methods.iter()? {
            let tuple = tuple?;
            let node = &tuple[0];
            if let Some(idx) = inv_indices.get(node) {
                targets.insert(*idx);
            }
        }

        if targets.is_empty() {
            return Ok(());
        }

        // Store the source alongside the ClassSource and get indices for the class
        let mut sources = BTreeSet::new();
        for tuple in start_methods.iter()? {
            let tuple = tuple?;
            let class_name = &tuple[0];
            let source = &tuple[1];
            if let Some(class_idx) = inv_indices.get(class_name) {
                sources.insert(ClassSource {
                    class: *class_idx,
                    source: match source {
                        DataValue::Str(v) => v.to_string(),
                        _ => bail!("invalid value for source: {}", source),
                    },
                });
            }
        }

        if sources.is_empty() {
            return Ok(());
        }

        log::trace!(
            "FindPathsDFS, depth = {}, nsources = {}, ntargets = {}",
            max_depth,
            sources.len(),
            targets.len()
        );

        let do_search = |start: ClassSource| -> Result<(String, Vec<Vec<u32>>)> {
            let mut seen = BTreeSet::new();
            let mut stack: Vec<(u32, usize)> = Vec::new();
            let mut paths: Vec<Vec<u32>> = Vec::new();
            let mut backpointers: Vec<u32> = vec![Default::default(); max_depth + 1];

            stack.push((start.class, 0));

            while let Some((candidate, depth)) = stack.pop() {
                // A real recusion check, if the candidate is already in backpointers up to
                // this depth we've hit a cycle. I think this should be cheaper in general than
                // going down that path?

                let current_path = &backpointers[0..depth];
                if current_path.contains(&candidate) {
                    continue;
                }

                backpointers[depth] = candidate;
                let depth = depth + 1;

                seen.clear();

                for target in graph.out_neighbors_with_values(candidate) {
                    let next_node = target.target;

                    // next_node == candidate is a simple recursion check, that
                    // won't catch all cycles, but should help here.
                    if next_node == candidate || !seen.insert(next_node) {
                        continue;
                    }

                    // Check if the call is to one of the targets, if so,
                    // great, add a path
                    if targets.contains(&next_node) {
                        // Keep using the candidate source since that's what we're ultimately
                        // interested in
                        backpointers[depth] = next_node;
                        let backslice = backpointers.as_slice();
                        let path = &backslice[0..depth + 1];
                        paths.push(path.into());
                        // We don't really want paths through a given target, so continue if we
                        // find one. Note we keep iterating though because we may find another
                        // target that goes elsewhere
                        continue;
                    }

                    if depth < max_depth {
                        stack.push((next_node, depth));
                    }

                    poison.check()?;
                }
            }

            Ok((start.source, paths))
        };

        let results = if sources.len() == 1 {
            if let Some(start) = sources.into_iter().nth(0) {
                vec![do_search(start)?]
            } else {
                Vec::new()
            }
        } else {
            let it = sources.into_par_iter();

            let res = it
                .map(do_search)
                .filter_map(|it| it.ok())
                .collect::<Vec<(String, Vec<Vec<u32>>)>>();

            poison.check()?;
            res
        };
        let mut v = Vec::with_capacity(4);
        for (source, paths) in results {
            let source_dv = DataValue::Str(source.into());
            v.push(source_dv);

            for path in paths {
                let from = *path.first().unwrap();
                let to = *path.last().unwrap();

                v.push(indices[from as usize].clone());
                v.push(indices[to as usize].clone());
                v.push(DataValue::List(
                    path.into_iter()
                        .map(|u| indices[u as usize].clone())
                        .collect::<Vec<DataValue>>(),
                ));
                out.put(v.clone());
                v.truncate(1);
            }
            v.truncate(0);
        }

        Ok(())
    }

    fn arity(
        &self,
        _options: &BTreeMap<SmartString<LazyCompact>, Expr>,
        _rule_head: &[Symbol],
        _span: SourceSpan,
    ) -> Result<usize> {
        Ok(4)
    }
}

pub(crate) struct LoadCallsCsv;

impl FixedRule for LoadCallsCsv {
    fn run(
        &self,
        payload: FixedRulePayload<'_, '_>,
        out: &'_ mut RegularTempStore,
        poison: Poison,
    ) -> Result<()> {
        let file_path = payload.string_option("file", None)?;
        let file = File::open(file_path.as_str()).into_diagnostic()?;
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .from_reader(file);

        for rec in reader.records() {
            poison.check()?;
            let record = rec.into_diagnostic()?;
            if record.len() != 6 {
                bail!(
                    "invalid method call CSV at {} nrows = {}",
                    file_path,
                    record.len()
                );
            }
            let from = format!(
                "{}->{}({})",
                record.get(0).unwrap(),
                record.get(1).unwrap(),
                record.get(2).unwrap()
            );
            let to = format!(
                "{}->{}({})",
                record.get(3).unwrap(),
                record.get(4).unwrap(),
                record.get(5).unwrap()
            );

            out.put(vec![DataValue::from(from), DataValue::from(to)]);
        }

        Ok(())
    }

    fn arity(
        &self,
        _options: &BTreeMap<SmartString<LazyCompact>, Expr>,
        _rule_head: &[Symbol],
        _span: SourceSpan,
    ) -> Result<usize> {
        Ok(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cozo::DbInstance;
    use rstest::*;

    fn setup_db(db: &DbInstance) {
        db.register_fixed_rule("FindPathsDFS".into(), FindPathsDFS {})
            .expect("failed to register FindPathsDFS");
        db.run_default(":create G { from: String, to: String, source: String }")
            .expect("failed to create G");
        db.run_default(
            r#"
?[from, to, source] <- [
    ['Z', 'P', 'A'],
    ['A', 'B', 'A'],
    ['A', 'C', 'A'],
    ['A', 'H', 'A'],

    ['B', 'D', 'A'],
    ['B', 'A', 'A'],
    ['B', 'F', 'A'],

    ['C', 'E', 'A'],
    ['C', 'F', 'A'],
    ['C', 'Q', 'A'],

    ['H', 'B', 'A'],

    ['D', 'T', 'A'],
    ['F', 'T', 'A'],


    ['Q', 'T', 'A'],
]

:put G { from, to, source }
    "#,
        )
        .expect("failed to setup db");
    }

    fn make_result_no_src(from: &str, to: &str, path: &[&str]) -> Vec<DataValue> {
        let mut values: Vec<DataValue> = Vec::with_capacity(path.len());

        for p in path {
            values.push((*p).into());
        }

        vec![from.into(), to.into(), DataValue::List(values)]
    }

    fn make_result(source: &str, from: &str, to: &str, path: &[&str]) -> Vec<DataValue> {
        let mut values: Vec<DataValue> = Vec::with_capacity(path.len());

        for p in path {
            values.push((*p).into());
        }

        vec![
            source.into(),
            from.into(),
            to.into(),
            DataValue::List(values),
        ]
    }

    #[rstest]
    fn test_find_dfs_no_start_one_end() {
        let db = DbInstance::default();
        setup_db(&db);

        let results = db
            .run_default(
                r#"
start[from, source] := *G[from, _, source]
end[] <- [['T']]
?[source, from, to, path] <~ FindPathsDFS(*G[], start[], end[], depth: 2)
"#,
            )
            .expect("failed to execute script");

        let mut expected = BTreeSet::new();
        for e in vec![
            make_result("A", "B", "T", &["B", "D", "T"]),
            make_result("A", "B", "T", &["B", "F", "T"]),
            make_result("A", "C", "T", &["C", "F", "T"]),
            make_result("A", "C", "T", &["C", "Q", "T"]),
            make_result("A", "D", "T", &["D", "T"]),
            make_result("A", "F", "T", &["F", "T"]),
            make_result("A", "Q", "T", &["Q", "T"]),
        ] {
            expected.insert(e);
        }

        let mut got = BTreeSet::new();

        for e in results.rows {
            got.insert(e);
        }

        assert_eq!(got, expected);

        let results = db
            .run_default(
                r#"
start[from, source] := *G[from, _, source]
end[] <- [['T']]
?[source, from, to, path] <~ FindPathsDFS(*G[], start[], end[], depth: 3)
"#,
            )
            .expect("failed to execute script");

        let mut expected = BTreeSet::new();
        for e in vec![
            make_result("A", "A", "T", &["A", "B", "D", "T"]),
            make_result("A", "A", "T", &["A", "B", "F", "T"]),
            make_result("A", "A", "T", &["A", "C", "F", "T"]),
            make_result("A", "A", "T", &["A", "C", "Q", "T"]),
            make_result("A", "H", "T", &["H", "B", "D", "T"]),
            make_result("A", "H", "T", &["H", "B", "F", "T"]),
            make_result("A", "B", "T", &["B", "D", "T"]),
            make_result("A", "B", "T", &["B", "F", "T"]),
            make_result("A", "C", "T", &["C", "F", "T"]),
            make_result("A", "C", "T", &["C", "Q", "T"]),
            make_result("A", "D", "T", &["D", "T"]),
            make_result("A", "F", "T", &["F", "T"]),
            make_result("A", "Q", "T", &["Q", "T"]),
        ] {
            expected.insert(e);
        }

        let mut got = BTreeSet::new();

        for e in results.rows {
            got.insert(e);
        }

        assert_eq!(got, expected);
    }

    #[rstest]
    fn test_find_dfs_no_start_two_ends() {
        let db = DbInstance::default();
        setup_db(&db);

        let results = db
            .run_default(
                r#"
start[from, source] := *G[from, _, source]
end[] <- [['T'], ['Q']]
?[source, from, to, path] <~ FindPathsDFS(*G[], start[], end[], depth: 2)
"#,
            )
            .expect("failed to execute script");

        let mut expected = BTreeSet::new();
        for e in vec![
            make_result("A", "B", "T", &["B", "D", "T"]),
            make_result("A", "B", "T", &["B", "F", "T"]),
            make_result("A", "C", "T", &["C", "F", "T"]),
            make_result("A", "D", "T", &["D", "T"]),
            make_result("A", "F", "T", &["F", "T"]),
            make_result("A", "Q", "T", &["Q", "T"]),
            make_result("A", "A", "Q", &["A", "C", "Q"]),
            make_result("A", "C", "Q", &["C", "Q"]),
        ] {
            expected.insert(e);
        }

        let mut got = BTreeSet::new();

        for e in results.rows {
            got.insert(e);
        }

        assert_eq!(got, expected);
    }

    #[rstest]
    fn test_find_dfs_no_paths() {
        let db = DbInstance::default();
        setup_db(&db);

        let results = db
            .run_default(
                r#"
start[from, source] := *G[from, _, source]
end[] <- [['Z']]
?[source, from, to, path] <~ FindPathsDFS(*G[], start[], end[])
"#,
            )
            .expect("failed to execute script");

        assert_eq!(results.rows.len(), 0);
    }

    #[rstest]
    fn test_find_dfs_limited_start() {
        let db = DbInstance::default();
        setup_db(&db);

        let results = db
            .run_default(
                r#"
start[] <- [['B', 'A']]
end[] <- [['T']]
?[source, from, to, path] <~ FindPathsDFS(*G[], start[], end[], depth: 2)
"#,
            )
            .expect("failed to execute script");

        let mut expected = BTreeSet::new();
        for e in vec![
            make_result("A", "B", "T", &["B", "D", "T"]),
            make_result("A", "B", "T", &["B", "F", "T"]),
        ] {
            expected.insert(e);
        }

        let mut got = BTreeSet::new();

        for e in results.rows {
            got.insert(e);
        }

        assert_eq!(got, expected);
    }

    #[rstest]
    fn test_find_reachable() {
        let db = DbInstance::default();
        db.register_fixed_rule("FindReachableDFS".into(), FindReachableDFS {})
            .expect("failed to register FindReachableDFS");
        db.run_default(":create G { from: String, to: String, source: String }")
            .expect("failed to create G");
        db.run_default(
            r#"
?[from, to, source] <- [
    ['Z', 'P', 'apk'],
    ['A', 'B', 'framework'],
    ['A', 'C', 'framework'],
    ['A', 'H', 'framework'],

    ['B', 'D', 'framework'],
    ['B', 'A', 'framework'],
    ['B', 'F', 'apk'],

    ['C', 'E', 'framework'],
    ['C', 'F', 'framework'],
    ['C', 'Q', 'apk'],

    ['H', 'B', 'framework'],

    ['D', 'T', 'framework'],
    ['F', 'T', 'apk'],


    ['Q', 'T', 'apk'],
]

:put G { from, to, source }
    "#,
        )
        .expect("failed to setup db");

        let results = db
            .run_default(
                r#"
source_filtered[from, to] := *G{from, to, source: "framework"}
start[] <- [["A"]]

?[from, to, path] <~ FindReachableDFS(source_filtered[from, to], start[], depth: 2)
"#,
            )
            .expect("failed to execute script");

        let mut expected = BTreeSet::new();
        for e in vec![
            make_result_no_src("A", "B", &["A", "B"]),
            make_result_no_src("A", "C", &["A", "C"]),
            make_result_no_src("A", "H", &["A", "H"]),
            make_result_no_src("A", "E", &["A", "C", "E"]),
            make_result_no_src("A", "F", &["A", "C", "F"]),
            make_result_no_src("A", "B", &["A", "H", "B"]),
            make_result_no_src("A", "D", &["A", "B", "D"]),
            make_result_no_src("A", "A", &["A", "B", "A"]),
        ] {
            expected.insert(e);
        }

        let mut got = BTreeSet::new();

        for e in results.rows {
            got.insert(e);
        }

        assert_eq!(got, expected);
    }

    #[rstest]
    fn test_find_dfs_src() {
        let db = DbInstance::default();
        db.register_fixed_rule("FindPathsDFS".into(), FindPathsDFS {})
            .expect("failed to register FindPathsDFS");
        db.run_default(
            r#"
?[from, to, source] <- [
    ['Z', 'P', 'apk'],
    ['A', 'B', 'framework'],
    ['A', 'C', 'framework'],
    ['A', 'H', 'framework'],

    ['B', 'D', 'framework'],
    ['B', 'A', 'framework'],
    ['B', 'F', 'apk'],

    ['C', 'E', 'framework'],
    ['C', 'F', 'framework'],
    ['C', 'Q', 'apk'],

    ['H', 'B', 'framework'],

    ['D', 'T', 'framework'],
    ['F', 'T', 'apk'],


    ['Q', 'T', 'apk'],
]

:create G { from: String, to: String, source: String }
    "#,
        )
        .expect("failed to setup db");

        let results = db
            .run_default(
                r#"
source_filtered[from, to] := *G{from, to, source: "framework"}
start[] <- [["A", "framework"]]
end[] <- [["T"]]

?[source, from, to, path] <~ FindPathsDFS(source_filtered[], start[], end[], depth: 3)
"#,
            )
            .expect("failed to execute script");

        let mut expected = BTreeSet::new();
        for e in vec![make_result("framework", "A", "T", &["A", "B", "D", "T"])] {
            expected.insert(e);
        }

        let mut got = BTreeSet::new();

        for e in results.rows {
            got.insert(e);
        }

        assert_eq!(got, expected);
    }
}

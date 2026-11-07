//! Save-file fuzzing over the real-save corpus: mutate a few fields of a save so it stays
//! plausible, load it the way the desktop does, and run it. A rejected save is fine; a panic
//! is a crash bug; a save that validates and then breaks invariants shows a validator gap.
//!
//! cargo test --release -p simtower-core fuzz_saves -- --ignored --nocapture
//! with DISCOVERY_SAVES=a:b:c and optionally DISCOVERY_CASES (default 300).

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use serde_json::Value;

use super::{FRAME_SECONDS, Monitor, frame, load_save};
use crate::SimulationSpeed;

#[derive(Clone, Debug)]
enum Mutation {
    /// Swap the values of two leaves that share a field name (two people's shaft ids, two
    /// cars' positions, ...).
    SwapSameField { first: usize, second: usize },
    /// Nudge a number: 0, -1, +-1, or a large value.
    Nudge { leaf: usize, how: u8 },
    /// Remove or duplicate an element of an array.
    Resize { array: usize, duplicate: bool },
}

fn mutation() -> impl Strategy<Value = Mutation> {
    prop_oneof![
        3 => (any::<usize>(), any::<usize>()).prop_map(|(first, second)| Mutation::SwapSameField { first, second }),
        2 => (any::<usize>(), 0u8..5).prop_map(|(leaf, how)| Mutation::Nudge { leaf, how }),
        2 => (any::<usize>(), any::<bool>()).prop_map(|(array, duplicate)| Mutation::Resize { array, duplicate }),
    ]
}

/// Paths into the traffic half of a save: leaves (with their field name) and arrays.
fn collect(
    value: &Value,
    path: &mut Vec<PathStep>,
    leaves: &mut Vec<(Vec<PathStep>, String)>,
    arrays: &mut Vec<Vec<PathStep>>,
    field: &str,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                path.push(PathStep::Key(key.clone()));
                collect(child, path, leaves, arrays, key);
                path.pop();
            }
        }
        Value::Array(items) => {
            arrays.push(path.clone());
            for (index, child) in items.iter().enumerate() {
                path.push(PathStep::Index(index));
                collect(child, path, leaves, arrays, field);
                path.pop();
            }
        }
        _ => leaves.push((path.clone(), field.to_owned())),
    }
}

#[derive(Clone, Debug)]
enum PathStep {
    Key(String),
    Index(usize),
}

fn at<'a>(root: &'a mut Value, path: &[PathStep]) -> &'a mut Value {
    path.iter().fold(root, |value, step| match step {
        PathStep::Key(key) => &mut value[key.as_str()],
        PathStep::Index(index) => &mut value[*index],
    })
}

fn apply(save: &mut Value, mutation: &Mutation) -> String {
    let traffic = &mut save["traffic"];
    let (mut leaves, mut arrays) = (Vec::new(), Vec::new());
    collect(
        traffic,
        &mut Vec::new(),
        &mut leaves,
        &mut arrays,
        "traffic",
    );
    match *mutation {
        Mutation::SwapSameField { first, second } => {
            let (path, field) = leaves[first % leaves.len()].clone();
            let peers = leaves
                .iter()
                .filter(|(_, name)| *name == field)
                .collect::<Vec<_>>();
            let (other, _) = peers[second % peers.len()].clone();
            let a = at(traffic, &path).clone();
            let b = at(traffic, &other).clone();
            *at(traffic, &path) = b.clone();
            *at(traffic, &other) = a.clone();
            format!("swap {field}: {a} <-> {b}")
        }
        Mutation::Nudge { leaf, how } => {
            let (path, field) = leaves[leaf % leaves.len()].clone();
            let target = at(traffic, &path);
            let Some(number) = target.as_f64() else {
                return format!("nudge skipped ({field} is {target})");
            };
            let nudged = match how {
                0 => 0.0,
                1 => -1.0,
                2 => number + 1.0,
                3 => number - 1.0,
                _ => 1.0e6,
            };
            *target = if target.is_f64() {
                Value::from(nudged)
            } else {
                Value::from(nudged as i64)
            };
            format!("nudge {field}: {number} -> {nudged}")
        }
        Mutation::Resize { array, duplicate } => {
            let path = arrays[array % arrays.len()].clone();
            let name = path.iter().rev().find_map(|step| match step {
                PathStep::Key(key) => Some(key.clone()),
                PathStep::Index(_) => None,
            });
            let Value::Array(items) = at(traffic, &path) else {
                unreachable!()
            };
            if items.is_empty() {
                return format!("resize skipped ({name:?} empty)");
            }
            let index = array % items.len();
            if duplicate {
                let copy = items[index].clone();
                items.insert(index, copy);
                format!("duplicate {name:?}[{index}]")
            } else {
                items.remove(index);
                format!("remove {name:?}[{index}]")
            }
        }
    }
}

#[test]
#[ignore = "needs DISCOVERY_SAVES"]
fn fuzz_saves() {
    let saves = std::env::var("DISCOVERY_SAVES").expect("DISCOVERY_SAVES");
    let corpus: Vec<(String, Value)> = saves
        .split(':')
        .map(|path| {
            (
                path.to_owned(),
                serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap(),
            )
        })
        .collect();
    let cases: usize = std::env::var("DISCOVERY_CASES").map_or(300, |cases| cases.parse().unwrap());
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let strategy = (0..corpus.len(), proptest::collection::vec(mutation(), 1..4));
    let (mut rejected, mut loaded, mut panicked) = (0, 0, 0);
    let mut gaps: BTreeMap<&'static str, (usize, String)> = BTreeMap::new();
    let mut panics: Vec<String> = Vec::new();
    std::panic::set_hook(Box::new(|_| {}));
    for _ in 0..cases {
        let (save_index, mutations) = strategy.new_tree(&mut runner).unwrap().current();
        let (path, original) = &corpus[save_index];
        let mut save = original.clone();
        let described = mutations
            .iter()
            .map(|mutation| apply(&mut save, mutation))
            .collect::<Vec<_>>();
        let json = save.to_string();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let Ok((mut tower, mut traffic)) = load_save(&json) else {
                return None;
            };
            let mut monitor = Monitor::default();
            let mut found = monitor.check(&tower, &traffic, 0.0);
            tower.clock.speed = SimulationSpeed::Tenfold;
            for _ in 0..600 {
                let dt = frame(&mut tower, &mut traffic, FRAME_SECONDS);
                found.extend(monitor.check(&tower, &traffic, dt));
            }
            Some(found)
        }));
        match outcome {
            Ok(None) => rejected += 1,
            Ok(Some(found)) => {
                loaded += 1;
                for violation in found {
                    gaps.entry(violation.kind)
                        .or_insert_with(|| {
                            (0, format!("{path}: {described:?} -> {}", violation.detail))
                        })
                        .0 += 1;
                }
            }
            Err(payload) => {
                panicked += 1;
                let message = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| {
                        payload
                            .downcast_ref::<&str>()
                            .map(|text| (*text).to_owned())
                    })
                    .unwrap_or_default();
                if panics.len() < 8 {
                    panics.push(format!("{path}: {described:?} -> panic: {message}"));
                }
            }
        }
    }
    let _ = std::panic::take_hook();
    println!("{cases} fuzzed saves: {rejected} rejected, {loaded} loaded, {panicked} panicked");
    for panic in &panics {
        println!("  PANIC {panic}");
    }
    for (kind, (count, example)) in &gaps {
        println!(
            "  validated but {kind}: {count} cases, e.g. {}",
            example.chars().take(400).collect::<String>()
        );
    }
}

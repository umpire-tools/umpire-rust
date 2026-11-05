use std::collections::{HashMap, HashSet};

use crate::error::{Result, UmpireError};
use crate::rule::Rule;

// DependencyGraph is the field dependency graph used for topological
// ordering. Mirrors the TS core's graph.ts.
pub(crate) struct DependencyGraph {
    nodes: Vec<String>,
    adjacency: HashMap<String, Vec<String>>,
    incoming_counts: HashMap<String, usize>,
}

pub(crate) fn build_graph(field_order: &[String], rules: &[Rule]) -> DependencyGraph {
    let mut g = DependencyGraph {
        nodes: field_order.to_vec(),
        adjacency: HashMap::new(),
        incoming_counts: HashMap::new(),
    };
    for node in field_order {
        g.adjacency.insert(node.clone(), Vec::new());
        g.incoming_counts.insert(node.clone(), 0);
    }

    let mut seen_edges: HashSet<String> = HashSet::new();
    let mut add_edge = |from: &str, to: &str, rule_type: &str| {
        let key = format!("{}:{}:{}", from, to, rule_type);
        if !seen_edges.insert(key) {
            return;
        }
        g.adjacency
            .entry(from.to_string())
            .or_default()
            .push(to.to_string());
        *g.incoming_counts.entry(to.to_string()).or_insert(0) += 1;
    };

    for rule in rules {
        if rule.kind == crate::rule::Kind::OneOf {
            // Branch resolution is internal; oneOf creates no ordering edges.
            continue;
        }
        let (ordering, _) = rule.graph_sources();
        for source in &ordering {
            for target in &rule.targets {
                if source == target {
                    continue;
                }
                add_edge(source, target, rule.kind.as_str());
            }
        }
    }
    g
}

impl DependencyGraph {
    // detectCycles returns an error naming a cycle, if one exists.
    pub(crate) fn detect_cycles(&self) -> Result<()> {
        fn visit(
            node: &str,
            g: &DependencyGraph,
            visited: &mut HashSet<String>,
            active: &mut HashSet<String>,
            path: &mut Vec<String>,
        ) -> Option<Vec<String>> {
            visited.insert(node.to_string());
            active.insert(node.to_string());
            path.push(node.to_string());

            if let Some(nexts) = g.adjacency.get(node) {
                for next in nexts {
                    if !visited.contains(next) {
                        if let Some(cycle) = visit(next, g, visited, active, path) {
                            return Some(cycle);
                        }
                        continue;
                    }
                    if !active.contains(next) {
                        continue;
                    }
                    let start = path.iter().position(|n| n == next).unwrap_or(0);
                    let mut cycle: Vec<String> = path[start..].to_vec();
                    cycle.push(next.clone());
                    return Some(cycle);
                }
            }

            path.pop();
            active.remove(node);
            None
        }

        let mut visited = HashSet::new();
        let mut active = HashSet::new();
        let mut path = Vec::new();
        for node in &self.nodes {
            if visited.contains(node) {
                continue;
            }
            if let Some(cycle) = visit(node, self, &mut visited, &mut active, &mut path) {
                return Err(UmpireError::new(format!(
                    "[@umpire/core] Cycle detected: {}",
                    cycle.join(" → ")
                )));
            }
        }
        Ok(())
    }

    // topologicalSort produces the evaluation order, seeding the queue in
    // field order for deterministic output.
    pub(crate) fn topological_sort(&self) -> Result<Vec<String>> {
        let mut incoming: HashMap<String, usize> = self.incoming_counts.clone();

        let mut queue: Vec<String> = self
            .nodes
            .iter()
            .filter(|f| incoming[*f] == 0)
            .cloned()
            .collect();

        let mut result = Vec::new();
        let mut index = 0;
        while index < queue.len() {
            let node = queue[index].clone();
            result.push(node.clone());
            index += 1;
            if let Some(nexts) = self.adjacency.get(&node) {
                for next in nexts {
                    let count = incoming.get_mut(next).unwrap();
                    *count -= 1;
                    if *count == 0 {
                        queue.push(next.clone());
                    }
                }
            }
        }

        if result.len() != self.nodes.len() {
            self.detect_cycles()?;
            return Err(UmpireError::new(
                "[@umpire/core] Unable to produce topological order",
            ));
        }
        Ok(result)
    }
}

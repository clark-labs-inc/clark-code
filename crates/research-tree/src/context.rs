use std::collections::BTreeSet;

use crate::{Node, ResearchTree, TreeError};

impl ResearchTree {
    /// Selected work first, then its nearest-to-farthest ancestors, followed by
    /// related parents, dependencies, and crosslinks in stable breadth-first
    /// order. Unrelated branches never displace the requested branch. This is
    /// bounded node context, not token budgeting or evidence verification.
    pub fn context(&self, key: &str, limit: usize) -> Result<Vec<Node>, TreeError> {
        if !(1..=32).contains(&limit) {
            return Err(TreeError::Invalid("context limit must be 1..32".into()));
        }
        if !self.nodes.contains_key(key) {
            return Err(TreeError::UnknownNode(key.into()));
        }
        let mut context = Vec::new();
        let mut seen = BTreeSet::new();
        let mut ancestor = Some(key.to_string());
        while let Some(key) = ancestor {
            if !seen.insert(key.clone()) || context.len() == limit {
                break;
            }
            let node = self.nodes.get(&key).expect("validated graph reference");
            ancestor = node.parent.clone();
            context.push(node.clone());
        }
        let mut cursor = 0;
        while cursor < context.len() && context.len() < limit {
            let node = &context[cursor];
            let mut links = node.links.clone();
            links.sort();
            let adjacent: Vec<_> = node
                .parent
                .iter()
                .chain(&node.dependencies)
                .chain(&links)
                .cloned()
                .collect();
            for key in adjacent {
                if seen.insert(key.clone()) {
                    context.push(
                        self.nodes
                            .get(&key)
                            .expect("validated graph reference")
                            .clone(),
                    );
                    if context.len() == limit {
                        break;
                    }
                }
            }
            cursor += 1;
        }
        Ok(context)
    }
}

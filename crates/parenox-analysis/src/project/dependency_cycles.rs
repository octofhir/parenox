//! Strongly connected components in the part of the graph blocked during scheduling.

pub(super) struct BlockedDependencies {
    pub(super) dependencies: Vec<Vec<usize>>,
    pub(super) cycle: Vec<Option<usize>>,
}

pub(super) fn classify(consumers: &[Vec<usize>], built: &[bool]) -> BlockedDependencies {
    let mut dependencies = vec![Vec::new(); consumers.len()];
    for (provider, edges) in consumers.iter().enumerate() {
        if !built[provider] {
            for &consumer in edges.iter().filter(|&&consumer| !built[consumer]) {
                dependencies[consumer].push(provider);
            }
        }
    }

    // Kosaraju's algorithm. Keep the DFS on the heap for long dependency chains.
    let mut visited = built.to_vec();
    let mut finished = Vec::new();
    let mut walk = Vec::new();
    for (root, edges) in consumers.iter().enumerate() {
        if std::mem::replace(&mut visited[root], true) {
            continue;
        }
        walk.push((root, edges.iter()));
        while let Some((node, edges)) = walk.last_mut() {
            if let Some(&next) = edges.next() {
                if !std::mem::replace(&mut visited[next], true) {
                    walk.push((next, consumers[next].iter()));
                }
            } else {
                finished.push(*node);
                walk.pop();
            }
        }
    }

    let mut cycle = vec![None; consumers.len()];
    visited.fill(false);
    let mut pending = Vec::new();
    let mut component = Vec::new();
    for root in finished.into_iter().rev() {
        if std::mem::replace(&mut visited[root], true) {
            continue;
        }
        pending.push(root);
        component.clear();
        while let Some(node) = pending.pop() {
            component.push(node);
            for &next in &dependencies[node] {
                if !std::mem::replace(&mut visited[next], true) {
                    pending.push(next);
                }
            }
        }
        if component.len() > 1 || dependencies[root].contains(&root) {
            for &node in &component {
                cycle[node] = Some(root);
            }
        }
    }
    BlockedDependencies {
        dependencies,
        cycle,
    }
}

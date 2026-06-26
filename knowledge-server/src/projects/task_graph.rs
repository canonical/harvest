use std::collections::{HashMap, HashSet};

#[derive(Clone)]
pub struct TaskNode {
    pub id:         String,
    pub name:       String,
    pub prompt:     String,
    pub depends_on: Vec<String>,
}

pub fn collect_subgraph(target_id: &str, all_tasks: Vec<TaskNode>) -> Vec<TaskNode> {
    let by_id: HashMap<&str, usize> = all_tasks.iter().enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();

    let mut visited: HashSet<String> = HashSet::new();
    let mut stack = vec![target_id.to_string()];

    while let Some(id) = stack.pop() {
        if visited.insert(id.clone()) {
            if let Some(&idx) = by_id.get(id.as_str()) {
                for dep in &all_tasks[idx].depends_on {
                    if !visited.contains(dep) {
                        stack.push(dep.clone());
                    }
                }
            }
        }
    }

    all_tasks.into_iter().filter(|t| visited.contains(&t.id)).collect()
}

pub fn compute_in_degrees(tasks: &[TaskNode]) -> HashMap<String, usize> {
    let ids: HashSet<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
    tasks.iter()
        .map(|t| {
            let count = t.depends_on.iter().filter(|d| ids.contains(d.as_str())).count();
            (t.id.clone(), count)
        })
        .collect()
}

pub fn compute_dependents(tasks: &[TaskNode]) -> HashMap<String, Vec<String>> {
    let ids: HashSet<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
    let mut map: HashMap<String, Vec<String>> =
        tasks.iter().map(|t| (t.id.clone(), Vec::new())).collect();
    for t in tasks {
        for dep in &t.depends_on {
            if ids.contains(dep.as_str()) {
                map.entry(dep.clone()).or_default().push(t.id.clone());
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, deps: &[&str]) -> TaskNode {
        TaskNode { id: id.into(), name: id.into(), prompt: id.into(),
                   depends_on: deps.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn single_node_subgraph() {
        let tasks = vec![node("a", &[])];
        let sub = collect_subgraph("a", tasks);
        assert_eq!(sub.len(), 1);
        assert_eq!(sub[0].id, "a");
    }

    #[test]
    fn linear_chain_subgraph() {
        let tasks = vec![node("a", &[]), node("b", &["a"]), node("c", &["b"])];
        let mut sub = collect_subgraph("c", tasks);
        sub.sort_by(|x, y| x.id.cmp(&y.id));
        assert_eq!(sub.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["a","b","c"]);
    }

    #[test]
    fn excludes_unreachable_tasks() {
        let tasks = vec![node("a", &[]), node("b", &[]), node("c", &["a"])];
        let sub = collect_subgraph("c", tasks);
        assert_eq!(sub.len(), 2);
        assert!(sub.iter().any(|t| t.id == "a"));
        assert!(sub.iter().any(|t| t.id == "c"));
    }

    #[test]
    fn in_degrees_linear() {
        let tasks = vec![node("a", &[]), node("b", &["a"]), node("c", &["b"])];
        let deg = compute_in_degrees(&tasks);
        assert_eq!(deg["a"], 0);
        assert_eq!(deg["b"], 1);
        assert_eq!(deg["c"], 1);
    }

    #[test]
    fn in_degrees_diamond() {
        let tasks = vec![node("a",&[]), node("b",&["a"]), node("c",&["a"]), node("d",&["b","c"])];
        let deg = compute_in_degrees(&tasks);
        assert_eq!(deg["a"], 0);
        assert_eq!(deg["b"], 1);
        assert_eq!(deg["c"], 1);
        assert_eq!(deg["d"], 2);
    }

    #[test]
    fn dependents_linear() {
        let tasks = vec![node("a", &[]), node("b", &["a"]), node("c", &["b"])];
        let deps = compute_dependents(&tasks);
        assert_eq!(deps["a"], vec!["b"]);
        assert_eq!(deps["b"], vec!["c"]);
        assert!(deps["c"].is_empty());
    }
}

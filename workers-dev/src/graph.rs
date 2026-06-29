use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct WorkerGraph {
    workers: Vec<String>,
    deps: HashMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct WorkerYaml {
    name: Option<String>,
    dependencies: Option<HashMap<String, String>>,
}

impl WorkerGraph {
    pub fn load(repo_root: &Path, workers: &[String]) -> Result<Self> {
        let managed: HashSet<&str> = workers.iter().map(String::as_str).collect();
        let mut deps = HashMap::new();

        for worker in workers {
            let yaml_path = repo_root.join(worker).join("iii.worker.yaml");
            let raw = std::fs::read_to_string(&yaml_path)
                .with_context(|| format!("read {}", yaml_path.display()))?;
            let parsed: WorkerYaml = serde_yaml::from_str(&raw)
                .with_context(|| format!("parse {}", yaml_path.display()))?;

            if let Some(name) = &parsed.name {
                if name != worker {
                    bail!("worker folder {worker} has iii.worker.yaml name={name} (mismatch)");
                }
            }

            let worker_deps: Vec<String> = parsed
                .dependencies
                .unwrap_or_default()
                .into_keys()
                .filter(|dep| managed.contains(dep.as_str()))
                .collect();

            deps.insert(worker.clone(), worker_deps);
        }

        Ok(Self {
            workers: workers.to_vec(),
            deps,
        })
    }

    pub fn workers(&self) -> &[String] {
        &self.workers
    }

    pub fn dependencies(&self, worker: &str) -> &[String] {
        self.deps.get(worker).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Topological start order (dependencies before dependents).
    pub fn topo_start_order(&self, subset: &[String]) -> Result<Vec<String>> {
        let mut deduped: Vec<&String> = Vec::new();
        let mut seen = HashSet::new();
        for worker in subset {
            if !self.workers.iter().any(|w| w == worker) {
                bail!("unknown worker {worker}");
            }
            if seen.insert(worker.as_str()) {
                deduped.push(worker);
            }
        }
        let subset_set: HashSet<&str> = deduped.iter().map(|w| w.as_str()).collect();
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

        for worker in &deduped {
            in_degree.entry(worker.as_str()).or_insert(0);
            for dep in self.dependencies(worker) {
                if !subset_set.contains(dep.as_str()) {
                    continue;
                }
                *in_degree.entry(worker.as_str()).or_insert(0) += 1;
                adj.entry(dep.as_str()).or_default().push(worker.as_str());
            }
        }

        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(name, _)| *name)
            .collect();
        queue.make_contiguous().sort_unstable();

        let mut order = Vec::with_capacity(deduped.len());
        while let Some(node) = queue.pop_front() {
            order.push(node.to_string());
            if let Some(children) = adj.get(node) {
                let mut next = Vec::new();
                for child in children {
                    let entry = in_degree.get_mut(child).expect("child in subset");
                    *entry -= 1;
                    if *entry == 0 {
                        next.push(*child);
                    }
                }
                next.sort_unstable();
                for child in next {
                    queue.push_back(child);
                }
            }
        }

        if order.len() != deduped.len() {
            bail!("dependency cycle detected among workers");
        }

        Ok(order)
    }

    /// Reverse topological stop order (dependents before dependencies).
    pub fn topo_stop_order(&self, subset: &[String]) -> Result<Vec<String>> {
        let mut order = self.topo_start_order(subset)?;
        order.reverse();
        Ok(order)
    }

    /// All managed workers that transitively depend on `worker` (excluding `worker`).
    pub fn reverse_dependents(&self, worker: &str) -> Result<Vec<String>> {
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([worker.to_string()]);

        while let Some(current) = queue.pop_front() {
            for candidate in &self.workers {
                if self.dependencies(candidate).contains(&current) && seen.insert(candidate.clone())
                {
                    queue.push_back(candidate.clone());
                }
            }
        }

        seen.remove(worker);
        let mut dependents: Vec<String> = seen.into_iter().collect();
        dependents.sort_unstable();
        Ok(dependents)
    }

    /// Closure for restart: worker + all transitive dependents, in start order.
    pub fn restart_closure(&self, worker: &str) -> Result<Vec<String>> {
        if !self.workers.iter().any(|w| w == worker) {
            bail!("unknown worker {worker}");
        }
        let mut affected: Vec<String> = vec![worker.to_string()];
        affected.extend(self.reverse_dependents(worker)?);
        self.topo_start_order(&affected)
    }

    /// Expand requested workers with missing dependencies (within managed set).
    pub fn closure_with_deps(&self, requested: &[String]) -> Result<Vec<String>> {
        let mut seen = HashSet::new();
        let mut queue: VecDeque<String> = requested.iter().cloned().collect();

        while let Some(worker) = queue.pop_front() {
            if !self.workers.iter().any(|w| w == &worker) {
                bail!("unknown worker {worker}");
            }
            if !seen.insert(worker.clone()) {
                continue;
            }
            for dep in self.dependencies(&worker) {
                if !seen.contains(dep) {
                    queue.push_back(dep.clone());
                }
            }
        }

        let subset: Vec<String> = seen.into_iter().collect();
        self.topo_start_order(&subset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_worker_yaml(dir: &Path, name: &str, deps: &[&str]) {
        let worker_dir = dir.join(name);
        fs::create_dir_all(&worker_dir).unwrap();
        let mut yaml = format!(
            "iii: v1\nname: {name}\nlanguage: rust\ndeploy: binary\nmanifest: Cargo.toml\nbin: {name}\ndescription: test\n"
        );
        if !deps.is_empty() {
            yaml.push_str("dependencies:\n");
            for dep in deps {
                yaml.push_str(&format!("  {dep}: \"^1.0.0\"\n"));
            }
        }
        fs::write(worker_dir.join("iii.worker.yaml"), yaml).unwrap();
    }

    fn fixture_repo() -> TempDir {
        let tmp = TempDir::new().unwrap();
        write_worker_yaml(tmp.path(), "session-manager", &[]);
        write_worker_yaml(tmp.path(), "llm-router", &[]);
        write_worker_yaml(tmp.path(), "context-manager", &["llm-router"]);
        write_worker_yaml(tmp.path(), "provider-anthropic", &["llm-router"]);
        write_worker_yaml(tmp.path(), "provider-openai", &["llm-router"]);
        write_worker_yaml(tmp.path(), "approval-gate", &["session-manager"]);
        write_worker_yaml(
            tmp.path(),
            "harness",
            &[
                "session-manager",
                "context-manager",
                "approval-gate",
                "provider-anthropic",
                "provider-openai",
            ],
        );
        tmp
    }

    #[test]
    fn topo_start_order_respects_dependencies() {
        let tmp = fixture_repo();
        let workers: Vec<String> = [
            "session-manager",
            "llm-router",
            "context-manager",
            "provider-anthropic",
            "provider-openai",
            "approval-gate",
            "harness",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        let graph = WorkerGraph::load(tmp.path(), &workers).unwrap();
        let order = graph.topo_start_order(&workers).unwrap();

        fn idx(order: &[String], name: &str) -> usize {
            order.iter().position(|w| w == name).unwrap()
        }

        assert!(idx(&order, "llm-router") < idx(&order, "context-manager"));
        assert!(idx(&order, "session-manager") < idx(&order, "approval-gate"));
        assert!(idx(&order, "context-manager") < idx(&order, "harness"));
    }

    #[test]
    fn reverse_dependents_of_llm_router() {
        let tmp = fixture_repo();
        let workers: Vec<String> = [
            "session-manager",
            "llm-router",
            "context-manager",
            "provider-anthropic",
            "provider-openai",
            "approval-gate",
            "harness",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        let graph = WorkerGraph::load(tmp.path(), &workers).unwrap();
        let deps = graph.reverse_dependents("llm-router").unwrap();
        assert!(deps.contains(&"context-manager".to_string()));
        assert!(deps.contains(&"provider-anthropic".to_string()));
        assert!(deps.contains(&"provider-openai".to_string()));
        assert!(deps.contains(&"harness".to_string()));
    }

    #[test]
    fn restart_closure_includes_self_and_dependents() {
        let tmp = fixture_repo();
        let workers: Vec<String> = [
            "session-manager",
            "llm-router",
            "context-manager",
            "provider-anthropic",
            "provider-openai",
            "approval-gate",
            "harness",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        let graph = WorkerGraph::load(tmp.path(), &workers).unwrap();
        let closure = graph.restart_closure("llm-router").unwrap();
        assert_eq!(closure.first().map(String::as_str), Some("llm-router"));
        assert!(closure.contains(&"harness".to_string()));
    }

    #[test]
    fn topo_start_order_rejects_unknown_worker() {
        let tmp = fixture_repo();
        let workers: Vec<String> = ["session-manager", "llm-router"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let graph = WorkerGraph::load(tmp.path(), &workers).unwrap();
        let err = graph
            .topo_start_order(&["session-manager".to_string(), "typo".to_string()])
            .unwrap_err();
        assert!(err.to_string().contains("unknown worker typo"));
    }

    #[test]
    fn topo_start_order_deduplicates_subset() {
        let tmp = fixture_repo();
        let workers: Vec<String> = ["session-manager", "llm-router"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let graph = WorkerGraph::load(tmp.path(), &workers).unwrap();
        let order = graph
            .topo_start_order(&[
                "session-manager".to_string(),
                "session-manager".to_string(),
                "llm-router".to_string(),
            ])
            .unwrap();
        assert_eq!(order.len(), 2);
        assert!(order.contains(&"session-manager".to_string()));
        assert!(order.contains(&"llm-router".to_string()));
    }
}

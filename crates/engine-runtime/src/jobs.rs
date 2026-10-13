use crossbeam_channel::{bounded, Receiver, Sender};
use petgraph::graph::{DiGraph, NodeIndex};

#[derive(Clone, Debug)]
pub struct JobDesc {
    pub name: String,
    pub cost_hint_ms: f32,
}

pub struct JobSystem {
    graph: DiGraph<JobDesc, ()>,
    tx: Sender<JobDesc>,
    rx: Receiver<JobDesc>,
    runtime: tokio::runtime::Runtime,
}

impl JobSystem {
    pub fn new(queue_capacity: usize) -> Self {
        let (tx, rx) = bounded(queue_capacity);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("engine-job")
            .enable_all()
            .build()
            .expect("tokio job runtime builds");
        Self {
            graph: DiGraph::<JobDesc, ()>::new(),
            tx,
            rx,
            runtime,
        }
    }

    pub fn add_job(&mut self, desc: JobDesc) -> NodeIndex {
        self.graph.add_node(desc)
    }

    pub fn add_dependency(&mut self, before: NodeIndex, after: NodeIndex) {
        self.graph.add_edge(before, after, ());
    }

    pub fn job_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn is_dag(&self) -> bool {
        petgraph::algo::is_cyclic_directed(&self.graph).not()
    }

    pub fn submit(&self, desc: JobDesc) -> bool {
        self.tx.try_send(desc).is_ok()
    }

    pub fn drain(&self) -> Vec<JobDesc> {
        let mut out = Vec::new();
        while let Ok(job) = self.rx.try_recv() {
            out.push(job);
        }
        out
    }

    pub fn block_on<F, T>(&self, fut: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        self.runtime.block_on(fut)
    }
}

trait BoolExt {
    fn not(self) -> bool;
}

impl BoolExt for bool {
    fn not(self) -> bool {
        !self
    }
}

pub fn directed_graph_example() -> DiGraph<&'static str, ()> {
    let mut graph = DiGraph::<&str, ()>::new();
    let sim = graph.add_node("sim");
    let physics = graph.add_node("physics");
    let extract = graph.add_node("extract");
    graph.add_edge(sim, physics, ());
    graph.add_edge(physics, extract, ());
    graph
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dag_construction_and_queue_roundtrip() {
        let mut system = JobSystem::new(8);
        let a = system.add_job(JobDesc {
            name: "sim".to_string(),
            cost_hint_ms: 4.0,
        });
        let b = system.add_job(JobDesc {
            name: "physics".to_string(),
            cost_hint_ms: 3.0,
        });
        system.add_dependency(a, b);
        assert_eq!(system.job_count(), 2);
        assert!(system.is_dag());
        assert!(system.submit(JobDesc {
            name: "asset-decode".to_string(),
            cost_hint_ms: 1.0,
        }));
        let drained = system.drain();
        assert_eq!(drained.len(), 1);
    }
}

use petgraph::stable_graph::{NodeIndex, StableDiGraph};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PassId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TargetMask(pub u32);

impl TargetMask {
    pub const DESKTOP: Self = Self(0b001);
    pub const WEB: Self = Self(0b010);
    pub const MOBILE: Self = Self(0b100);
    pub const ALL: Self = Self(0b111);
    pub const EMPTY: Self = Self(0);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl std::ops::BitOr for TargetMask {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitAnd for TargetMask {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceAccess {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceUsage {
    pub name: &'static str,
    pub access: ResourceAccess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceEdge {
    pub resource: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassKind {
    Graphics,
    Compute,
}

pub struct RenderContext {
    pub frame_index: u64,
    pub target: TargetMask,
    pub subgroups_off: bool,
}

pub struct RenderNode {
    pub name: &'static str,
    pub reads: Vec<ResourceUsage>,
    pub writes: Vec<ResourceUsage>,
    pub enabled_on: TargetMask,
    pub kind: PassKind,
    pub exec: fn(&mut RenderContext),
}

impl std::fmt::Debug for RenderNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderNode")
            .field("name", &self.name)
            .field("reads", &self.reads)
            .field("writes", &self.writes)
            .field("enabled_on", &self.enabled_on)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct TransientPool {
    pub capacity_bytes: usize,
}

impl Default for TransientPool {
    fn default() -> Self {
        Self {
            capacity_bytes: 256 << 20,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentPassSpec {
    pub name: &'static str,
    pub reads: Vec<ResourceUsage>,
    pub writes: Vec<ResourceUsage>,
    pub enabled_on: TargetMask,
    pub kind: PassKind,
    pub exec: fn(&mut RenderContext),
    pub after: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    DuplicatePass(&'static str),
    UnknownAnchor(&'static str),
    EmptyTargetMask,
    UnknownTargetBits(u32),
    WriteConflict {
        pass: &'static str,
        resource: &'static str,
        holder: &'static str,
    },
    CycleDetected,
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicatePass(name) => {
                write!(f, "E_GRAPH_DUPLICATE: pass '{name}' already registered")
            }
            Self::UnknownAnchor(name) => {
                write!(f, "E_GRAPH_ANCHOR: anchor pass '{name}' not found")
            }
            Self::EmptyTargetMask => write!(f, "E_GRAPH_TARGET: enabled_on mask is empty"),
            Self::UnknownTargetBits(bits) => {
                write!(f, "E_GRAPH_TARGET: unknown target bits {bits:#x}")
            }
            Self::WriteConflict {
                pass,
                resource,
                holder,
            } => write!(
                f,
                "E_GRAPH_ALIAS: pass '{pass}' writes '{resource}' already written by '{holder}'"
            ),
            Self::CycleDetected => write!(f, "E_GRAPH_CYCLE: pass insertion introduced a cycle"),
        }
    }
}

impl std::error::Error for GraphError {}

pub struct FrameGraph {
    pub graph: StableDiGraph<RenderNode, ResourceEdge>,
    pub order: Vec<NodeIndex>,
    pub transient_pool: TransientPool,
    next_id: u32,
}

fn noop(_: &mut RenderContext) {}

fn node(
    name: &'static str,
    reads: &[(&'static str, ResourceAccess)],
    writes: &[&'static str],
    enabled_on: TargetMask,
    kind: PassKind,
) -> RenderNode {
    RenderNode {
        name,
        reads: reads
            .iter()
            .map(|(name, access)| ResourceUsage {
                name,
                access: *access,
            })
            .collect(),
        writes: writes
            .iter()
            .map(|name| ResourceUsage {
                name,
                access: ResourceAccess::Write,
            })
            .collect(),
        enabled_on,
        kind,
        exec: noop,
    }
}

fn res(name: &'static str) -> (&'static str, ResourceAccess) {
    (name, ResourceAccess::Read)
}

impl FrameGraph {
    pub fn default_forward_plus() -> Self {
        let mut graph = StableDiGraph::new();
        // Extract -> depth_prepass -> cluster_lights (compute) -> opaque ->
        // sky/probes -> transparent -> taa -> post (tonemap/bloom) -> ui.
        // Deferred variant swaps opaque->{gbuffer, light_composite}; visibility
        // stays behind `--features visibility` (spec 03 §2.2).
        let passes = [
            node(
                "extract",
                &[],
                &["render_world"],
                TargetMask::ALL,
                PassKind::Graphics,
            ),
            node(
                "depth_prepass",
                &[res("render_world")],
                &["depth"],
                TargetMask::ALL,
                PassKind::Graphics,
            ),
            node(
                "cluster_lights",
                &[res("depth"), res("light_list")],
                &["cluster_grid"],
                TargetMask::ALL,
                PassKind::Compute,
            ),
            node(
                "opaque",
                &[
                    res("depth"),
                    res("cluster_grid"),
                    res("shadow_maps"),
                    res("probes"),
                ],
                &["hdr_color"],
                TargetMask::ALL,
                PassKind::Graphics,
            ),
            node(
                "sky_probes",
                &[res("hdr_color")],
                &["hdr_color"],
                TargetMask::ALL,
                PassKind::Graphics,
            ),
            node(
                "transparent",
                &[res("hdr_color"), res("depth")],
                &["hdr_color"],
                TargetMask::ALL,
                PassKind::Graphics,
            ),
            node(
                "taa",
                &[res("hdr_color"), res("velocity"), res("history")],
                &["hdr_color"],
                TargetMask::DESKTOP | TargetMask::MOBILE,
                PassKind::Compute,
            ),
            node(
                "post",
                &[res("hdr_color")],
                &["ldr_color"],
                TargetMask::ALL,
                PassKind::Compute,
            ),
            node(
                "ui",
                &[res("ldr_color")],
                &["swapchain"],
                TargetMask::ALL,
                PassKind::Graphics,
            ),
        ];
        let mut indices = Vec::with_capacity(passes.len());
        for pass in passes {
            indices.push(graph.add_node(pass));
        }
        for window in indices.windows(2) {
            let resource = graph[window[1]]
                .reads
                .first()
                .map(|usage| usage.name)
                .unwrap_or("order");
            graph.add_edge(window[0], window[1], ResourceEdge { resource });
        }
        let order = indices;
        Self {
            graph,
            order,
            transient_pool: TransientPool::default(),
            next_id: 100,
        }
    }

    pub fn pass_names(&self) -> Vec<&'static str> {
        self.order
            .iter()
            .map(|index| self.graph[*index].name)
            .collect()
    }

    pub fn active_passes(&self, target: TargetMask) -> Vec<&'static str> {
        self.order
            .iter()
            .filter(|index| self.graph[**index].enabled_on.intersects(target))
            .map(|index| self.graph[*index].name)
            .collect()
    }

    pub fn register_agent_pass(&mut self, spec: AgentPassSpec) -> Result<PassId, GraphError> {
        if spec.enabled_on == TargetMask::EMPTY {
            return Err(GraphError::EmptyTargetMask);
        }
        if spec.enabled_on.0 & !TargetMask::ALL.0 != 0 {
            return Err(GraphError::UnknownTargetBits(
                spec.enabled_on.0 & !TargetMask::ALL.0,
            ));
        }
        if self.graph.node_weights().any(|node| node.name == spec.name) {
            return Err(GraphError::DuplicatePass(spec.name));
        }
        for write in &spec.writes {
            for existing in self.graph.node_weights() {
                let clash = existing.writes.iter().any(|usage| usage.name == write.name);
                let both_always = existing.enabled_on.intersects(spec.enabled_on);
                if clash && both_always {
                    return Err(GraphError::WriteConflict {
                        pass: spec.name,
                        resource: write.name,
                        holder: existing.name,
                    });
                }
            }
        }
        let index = self.graph.add_node(RenderNode {
            name: spec.name,
            reads: spec.reads.clone(),
            writes: spec.writes.clone(),
            enabled_on: spec.enabled_on,
            kind: spec.kind,
            exec: spec.exec,
        });
        let anchor = match spec.after {
            Some(name) => self
                .graph
                .node_indices()
                .find(|candidate| self.graph[*candidate].name == name)
                .ok_or(GraphError::UnknownAnchor(name))?,
            None => self
                .order
                .last()
                .copied()
                .ok_or(GraphError::UnknownAnchor("tail"))?,
        };
        let edge_resource = spec
            .reads
            .first()
            .map(|usage| usage.name)
            .unwrap_or("order");
        self.graph.add_edge(
            anchor,
            index,
            ResourceEdge {
                resource: edge_resource,
            },
        );
        if petgraph::algo::is_cyclic_directed(&self.graph) {
            self.graph.remove_node(index);
            return Err(GraphError::CycleDetected);
        }
        self.order.push(index);
        let id = PassId(self.next_id);
        self.next_id += 1;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_noop(_: &mut RenderContext) {}

    #[test]
    fn default_order_matches_spec_phases() {
        let graph = FrameGraph::default_forward_plus();
        assert_eq!(
            graph.pass_names(),
            vec![
                "extract",
                "depth_prepass",
                "cluster_lights",
                "opaque",
                "sky_probes",
                "transparent",
                "taa",
                "post",
                "ui",
            ]
        );
    }

    #[test]
    fn web_skips_taa_but_keeps_post_and_ui() {
        let graph = FrameGraph::default_forward_plus();
        let web = graph.active_passes(TargetMask::WEB);
        assert!(!web.contains(&"taa"));
        assert!(web.contains(&"post"));
        assert!(web.contains(&"ui"));
    }

    #[test]
    fn agent_pass_validates_uniqueness_targets_and_aliasing() {
        let mut graph = FrameGraph::default_forward_plus();
        let id = graph
            .register_agent_pass(AgentPassSpec {
                name: "agent_outline",
                reads: vec![ResourceUsage {
                    name: "ldr_color",
                    access: ResourceAccess::Read,
                }],
                writes: vec![ResourceUsage {
                    name: "agent_mask",
                    access: ResourceAccess::Write,
                }],
                enabled_on: TargetMask::DESKTOP,
                kind: PassKind::Graphics,
                exec: agent_noop,
                after: Some("post"),
            })
            .expect("valid agent pass registers");
        assert!(graph.pass_names().contains(&"agent_outline"));
        let _ = id;

        let duplicate = graph.register_agent_pass(AgentPassSpec {
            name: "agent_outline",
            reads: vec![],
            writes: vec![],
            enabled_on: TargetMask::DESKTOP,
            kind: PassKind::Graphics,
            exec: agent_noop,
            after: None,
        });
        assert_eq!(duplicate, Err(GraphError::DuplicatePass("agent_outline")));

        let empty = graph.register_agent_pass(AgentPassSpec {
            name: "agent_empty",
            reads: vec![],
            writes: vec![],
            enabled_on: TargetMask::EMPTY,
            kind: PassKind::Graphics,
            exec: agent_noop,
            after: None,
        });
        assert_eq!(empty, Err(GraphError::EmptyTargetMask));

        let clash = graph.register_agent_pass(AgentPassSpec {
            name: "agent_clash",
            reads: vec![],
            writes: vec![ResourceUsage {
                name: "hdr_color",
                access: ResourceAccess::Write,
            }],
            enabled_on: TargetMask::ALL,
            kind: PassKind::Graphics,
            exec: agent_noop,
            after: None,
        });
        assert!(matches!(clash, Err(GraphError::WriteConflict { .. })));

        let unknown = graph.register_agent_pass(AgentPassSpec {
            name: "agent_lost",
            reads: vec![],
            writes: vec![],
            enabled_on: TargetMask::DESKTOP,
            kind: PassKind::Graphics,
            exec: agent_noop,
            after: Some("no_such_pass"),
        });
        assert_eq!(unknown, Err(GraphError::UnknownAnchor("no_such_pass")));
    }
}

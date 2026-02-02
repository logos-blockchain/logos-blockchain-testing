#[derive(Clone, Debug, Default)]
pub struct TopologyShapeBuilder {
    node_count: Option<usize>,
    star_network: bool,
}

impl TopologyShapeBuilder {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            node_count: None,
            star_network: false,
        }
    }

    #[must_use]
    pub const fn with_nodes(mut self, count: usize) -> Self {
        self.node_count = Some(count);
        self
    }

    #[must_use]
    pub const fn with_star_network(mut self) -> Self {
        self.star_network = true;
        self
    }

    #[must_use]
    pub fn node_count_or(&self, fallback: usize) -> usize {
        self.node_count.unwrap_or(fallback)
    }

    #[must_use]
    pub fn star_network_enabled(&self) -> bool {
        self.star_network
    }
}

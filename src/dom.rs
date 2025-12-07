//! The document tree.
//!
//! Nodes live in one append-only arena and refer to each other by [`NodeId`].
//! A parent is always created before its children, so a node's id is greater
//! than its parent's. The style pass relies on that ordering.

/// A handle to a node in a [`Document`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub(crate) usize);

/// An element's tag name and attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// The tag name, lowercased.
    pub name: String,
    /// Attributes in source order, with lowercased names.
    pub attributes: Vec<(String, String)>,
}

impl Element {
    /// The value of the named attribute, if present.
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// The whitespace-separated entries of the `class` attribute.
    pub fn classes(&self) -> impl Iterator<Item = &str> {
        self.attribute("class")
            .unwrap_or_default()
            .split_ascii_whitespace()
    }
}

/// What a node holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeData {
    /// The root of the tree.
    Document,
    /// An element.
    Element(Element),
    /// A run of text.
    Text(String),
}

/// One node of the tree.
#[derive(Debug, Clone)]
pub struct Node {
    /// The parent, absent only for the document root.
    pub parent: Option<NodeId>,
    /// Children in document order.
    pub children: Vec<NodeId>,
    /// The node's content.
    pub data: NodeData,
}

/// A parsed document.
#[derive(Debug, Clone)]
pub struct Document {
    nodes: Vec<Node>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// Creates a document that holds only its root.
    pub fn new() -> Self {
        Self {
            nodes: vec![Node {
                parent: None,
                children: Vec::new(),
                data: NodeData::Document,
            }],
        }
    }

    /// The document root.
    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    /// The number of nodes, which is also one past the largest [`NodeId`].
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the document holds nothing but its root.
    pub fn is_empty(&self) -> bool {
        self.nodes.len() == 1
    }

    /// The node behind a handle.
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0]
    }

    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.0]
    }

    /// The element behind a handle, or `None` for text and the root.
    pub fn element(&self, id: NodeId) -> Option<&Element> {
        match &self.node(id).data {
            NodeData::Element(element) => Some(element),
            _ => None,
        }
    }

    /// Adds a new last child to `parent`.
    pub fn append(&mut self, parent: NodeId, data: NodeData) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            parent: Some(parent),
            children: Vec::new(),
            data,
        });
        self.nodes[parent.0].children.push(id);
        id
    }

    /// Every node in document order, the root included.
    pub fn ids(&self) -> impl Iterator<Item = NodeId> + use<> {
        (0..self.nodes.len()).map(NodeId)
    }

    /// The first element with the given tag name, in creation order.
    pub fn find_element(&self, name: &str) -> Option<NodeId> {
        self.ids()
            .find(|&id| self.element(id).is_some_and(|element| element.name == name))
    }

    /// The concatenated text of a node and its descendants.
    pub fn text_content(&self, id: NodeId) -> String {
        let mut text = String::new();
        self.collect_text(id, &mut text);
        text
    }

    fn collect_text(&self, id: NodeId, out: &mut String) {
        let node = self.node(id);
        if let NodeData::Text(text) = &node.data {
            out.push_str(text);
        }
        for &child in &node.children {
            self.collect_text(child, out);
        }
    }
}

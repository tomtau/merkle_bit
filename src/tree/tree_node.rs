#[cfg(feature = "use_bincode")]
use bincode::{deserialize, serialize};
#[cfg(feature = "use_ron")]
use ron;
#[cfg(feature = "use_serde")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "use_cbor")]
use serde_cbor;
#[cfg(feature = "use_json")]
use serde_json;
#[cfg(feature = "use_pickle")]
use serde_pickle;
#[cfg(feature = "use_yaml")]
use serde_yaml;

#[cfg(feature = "use_serde")]
use crate::merkle_bit::BinaryMerkleTreeResult;
#[cfg(feature = "use_serialization")]
use crate::traits::{Decode, Encode};
use crate::traits::{Node, NodeVariant};
use crate::tree::tree_branch::TreeBranch;
use crate::tree::tree_data::TreeData;
use crate::tree::tree_leaf::TreeLeaf;
#[cfg(feature = "use_rayon")]
use evmap::ShallowCopy;

/// A node in the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(any(feature = "use_serde"), derive(Serialize, Deserialize))]
pub struct TreeNode {
    /// The number of references to this node.
    pub references: u64,
    /// The `NodeVariant` of the node.
    pub node: NodeVariant<TreeBranch, TreeLeaf, TreeData>,
}

impl TreeNode {
    /// Creates a new `TreeNode`.
    #[inline]
    pub const fn new(node_variant: NodeVariant<TreeBranch, TreeLeaf, TreeData>) -> Self {
        Self {
            references: 0,
            node: node_variant,
        }
    }

    /// Gets the number of references to the node.
    const fn get_references(&self) -> u64 {
        self.references
    }

    /// Sets the number of references to the node.
    fn set_references(&mut self, references: u64) {
        self.references = references;
    }

    /// Sets the node as a `NodeVariant::Branch`.
    fn set_branch(&mut self, branch: TreeBranch) {
        self.node = NodeVariant::Branch(branch);
    }

    /// Sets the node as a `NodeVariant::Leaf`.
    fn set_leaf(&mut self, leaf: TreeLeaf) {
        self.node = NodeVariant::Leaf(leaf);
    }

    /// Sets the node as a `NodeVariant::Data`.
    fn set_data(&mut self, data: TreeData) {
        self.node = NodeVariant::Data(data);
    }
    #[cfg(feature = "use_rayon")]
    unsafe fn from_raw(
        node_variant: *const NodeVariant<TreeBranch, TreeLeaf, TreeData>,
        references: u64,
    ) -> Self {
        let node = std::ptr::read(node_variant);
        Self { references, node }
    }
}

impl Node<TreeBranch, TreeLeaf, TreeData> for TreeNode {
    #[inline]
    fn new(node_variant: NodeVariant<TreeBranch, TreeLeaf, TreeData>) -> Self {
        Self::new(node_variant)
    }

    #[inline]
    fn get_references(&self) -> u64 {
        Self::get_references(self)
    }
    #[inline]
    fn get_variant(self) -> NodeVariant<TreeBranch, TreeLeaf, TreeData> {
        self.node
    }

    #[inline]
    fn set_references(&mut self, references: u64) {
        Self::set_references(self, references)
    }
    #[inline]
    fn set_branch(&mut self, branch: TreeBranch) {
        Self::set_branch(self, branch)
    }
    #[inline]
    fn set_leaf(&mut self, leaf: TreeLeaf) {
        Self::set_leaf(self, leaf)
    }
    #[inline]
    fn set_data(&mut self, data: TreeData) {
        Self::set_data(self, data)
    }
}

#[cfg(feature = "use_bincode")]
impl Encode for TreeNode {
    #[inline]
    fn encode(&self) -> BinaryMerkleTreeResult<Vec<u8>> {
        Ok(serialize(self)?)
    }
}

#[cfg(feature = "use_json")]
impl Encode for TreeNode {
    #[inline]
    fn encode(&self) -> BinaryMerkleTreeResult<Vec<u8>> {
        let encoded = serde_json::to_string(&self)?;
        Ok(encoded.as_bytes().to_vec())
    }
}

#[cfg(feature = "use_cbor")]
impl Encode for TreeNode {
    #[inline]
    fn encode(&self) -> BinaryMerkleTreeResult<Vec<u8>> {
        Ok(serde_cbor::to_vec(&self)?)
    }
}

#[cfg(feature = "use_yaml")]
impl Encode for TreeNode {
    #[inline]
    fn encode(&self) -> BinaryMerkleTreeResult<Vec<u8>> {
        Ok(serde_yaml::to_vec(&self)?)
    }
}

#[cfg(feature = "use_pickle")]
impl Encode for TreeNode {
    #[inline]
    fn encode(&self) -> BinaryMerkleTreeResult<Vec<u8>> {
        Ok(serde_pickle::to_vec(&self, true)?)
    }
}

#[cfg(feature = "use_ron")]
impl Encode for TreeNode {
    #[inline]
    fn encode(&self) -> BinaryMerkleTreeResult<Vec<u8>> {
        Ok(ron::ser::to_string(&self)?.as_bytes().to_vec())
    }
}

#[cfg(feature = "use_bincode")]
impl Decode for TreeNode {
    #[inline]
    fn decode(buffer: &[u8]) -> BinaryMerkleTreeResult<Self> {
        Ok(deserialize(buffer)?)
    }
}

#[cfg(feature = "use_json")]
impl Decode for TreeNode {
    #[inline]
    fn decode(buffer: &[u8]) -> BinaryMerkleTreeResult<Self> {
        let decoded_string = String::from_utf8(buffer.to_vec())?;
        let decoded = serde_json::from_str(&decoded_string)?;
        Ok(decoded)
    }
}

#[cfg(feature = "use_cbor")]
impl Decode for TreeNode {
    #[inline]
    fn decode(buffer: &[u8]) -> BinaryMerkleTreeResult<Self> {
        Ok(serde_cbor::from_slice(buffer)?)
    }
}

#[cfg(feature = "use_yaml")]
impl Decode for TreeNode {
    #[inline]
    fn decode(buffer: &[u8]) -> BinaryMerkleTreeResult<Self> {
        Ok(serde_yaml::from_slice(buffer)?)
    }
}

#[cfg(feature = "use_pickle")]
impl Decode for TreeNode {
    #[inline]
    fn decode(buffer: &[u8]) -> BinaryMerkleTreeResult<Self> {
        Ok(serde_pickle::from_slice(buffer)?)
    }
}

#[cfg(feature = "use_ron")]
impl Decode for TreeNode {
    #[inline]
    fn decode(buffer: &[u8]) -> BinaryMerkleTreeResult<Self> {
        Ok(ron::de::from_bytes(buffer)?)
    }
}

#[cfg(feature = "use_rayon")]
impl ShallowCopy for TreeNode {
    #[inline]
    unsafe fn shallow_copy(&mut self) -> Self {
        let raw_node = &self.node as *const _;
        Self::from_raw(raw_node, self.references)
    }
}

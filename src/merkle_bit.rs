#[cfg(not(any(feature = "use_hashbrown")))]
use std::collections::HashMap;
use std::collections::{HashSet, VecDeque};
use std::marker::PhantomData;
use std::path::PathBuf;

#[cfg(feature = "use_hashbrown")]
use hashbrown::HashMap;
#[cfg(feature = "use_rayon")]
use rayon::prelude::*;

use crate::constants::KEY_LEN;
use crate::traits::{
    Branch, Data, Database, Decode, Encode, Exception, Hasher, Leaf, Node, NodeVariant,
};
#[cfg(feature = "use_rayon")]
use crate::utils::merge_cell::MergeCell;
use crate::utils::tree_cell::TreeCell;
use crate::utils::tree_ref::TreeRef;
#[cfg(feature = "use_rayon")]
use crate::utils::tree_ref_raw::TreeRefRaw;
use crate::utils::tree_utils::{
    calc_min_split_index, check_descendants, fast_log_2, generate_leaf_map, split_pairs,
};

/// A generic `Result` from an operation involving a `MerkleBIT`
pub type BinaryMerkleTreeResult<T> = Result<T, Exception>;

/// The `MerkleBIT` structure relies on many specified types:
/// # Required Type Annotations
/// * **`DatabaseType`**: The type to use for database-like operations.  `DatabaseType` must implement the `Database` trait.
/// * **`BranchType`**: The type used for representing branches in the tree. `BranchType` must implement the `Branch` trait.
/// * **`LeafType`**: The type used for representing leaves in the tree.  `LeafType` must implement the `Leaf` trait.
/// * **`DataType`**: The type used for representing data nodes in the tree.  `DataType` must implement the `Data` trait.
/// * **`NodeType`**: The type used for the outer node that can be either a branch, leaf, or data.  `NodeType` must implement the `Node` trait.
/// * **`HasherType`**: The type of hasher to use for hashing locations on the tree.  `HasherType` must implement the `Hasher` trait.
/// * **`ValueType`**: The type to return from a get.  `ValueType` must implement the `Encode` and `Decode` traits.
/// # Properties
/// * **db**: The database to store and retrieve values
/// * **depth**: The maximum permitted depth of the tree.
#[cfg(not(feature = "use_rayon"))]
pub struct MerkleBIT<DatabaseType, BranchType, LeafType, DataType, NodeType, HasherType, ValueType>
where
    DatabaseType: Database<NodeType = NodeType>,
    BranchType: Branch,
    LeafType: Leaf,
    DataType: Data,
    NodeType: Node<BranchType, LeafType, DataType>,
    HasherType: Hasher,
    ValueType: Decode + Encode,
{
    /// The database to store tree nodes.
    db: DatabaseType,
    /// The maximum depth of the tree.
    depth: usize,
    /// Marker for dealing with `BranchType`.
    branch: PhantomData<BranchType>,
    /// Marker for dealing with `LeafType`.
    leaf: PhantomData<LeafType>,
    /// Marker for dealing with `DataType`.
    data: PhantomData<DataType>,
    /// Marker for dealing with `NodeType`.
    node: PhantomData<NodeType>,
    /// Marker for dealing with `HasherType`.
    hasher: PhantomData<HasherType>,
    /// Marker for dealing with `ValueType`.
    value: PhantomData<ValueType>,
}

#[cfg(feature = "use_rayon")]
pub struct MerkleBIT<DatabaseType, BranchType, LeafType, DataType, NodeType, HasherType, ValueType>
where
    DatabaseType: Database<NodeType = NodeType> + Send + Sync,
    BranchType: Branch,
    LeafType: Leaf,
    DataType: Data,
    NodeType: Node<BranchType, LeafType, DataType> + Send + Sync,
    HasherType: Hasher,
    ValueType: Decode + Encode + Sync + Send,
{
    db: DatabaseType,
    depth: usize,
    branch: PhantomData<*const BranchType>,
    leaf: PhantomData<*const LeafType>,
    data: PhantomData<*const DataType>,
    node: PhantomData<*const NodeType>,
    hasher: PhantomData<*const HasherType>,
    value: PhantomData<*const ValueType>,
}

#[cfg(not(feature = "use_rayon"))]
impl<DatabaseType, BranchType, LeafType, DataType, NodeType, HasherType, ValueType>
    MerkleBIT<DatabaseType, BranchType, LeafType, DataType, NodeType, HasherType, ValueType>
where
    DatabaseType: Database<NodeType = NodeType>,
    BranchType: Branch,
    LeafType: Leaf,
    DataType: Data,
    NodeType: Node<BranchType, LeafType, DataType>,
    HasherType: Hasher<HashType = HasherType>,
    ValueType: Decode + Encode,
{
    /// Create a new `MerkleBIT` from a saved database
    #[inline]
    pub fn new(path: &PathBuf, depth: usize) -> BinaryMerkleTreeResult<Self> {
        let db = DatabaseType::open(path)?;
        Ok(Self {
            db,
            depth,
            branch: PhantomData,
            leaf: PhantomData,
            data: PhantomData,
            node: PhantomData,
            hasher: PhantomData,
            value: PhantomData,
        })
    }

    /// Create a new `MerkleBIT` from an already opened database
    #[inline]
    pub fn from_db(db: DatabaseType, depth: usize) -> BinaryMerkleTreeResult<Self> {
        Ok(Self {
            db,
            depth,
            branch: PhantomData,
            leaf: PhantomData,
            data: PhantomData,
            node: PhantomData,
            hasher: PhantomData,
            value: PhantomData,
        })
    }

    /// Get items from the `MerkleBIT`.  Returns a map of `Option`s which may include the corresponding values.
    #[inline]
    pub fn get<'a>(
        &self,
        root_hash: &[u8; KEY_LEN],
        keys: &mut [&'a [u8; KEY_LEN]],
    ) -> BinaryMerkleTreeResult<HashMap<&'a [u8; KEY_LEN], Option<ValueType>>> {
        if keys.is_empty() {
            return Ok(HashMap::new());
        }

        let mut leaf_map = generate_leaf_map(keys);

        #[cfg(not(feature = "use_rayon"))]
        keys.sort();
        #[cfg(feature = "use_rayon")]
        keys.par_sort();

        let root_node;
        if let Some(n) = self.db.get_node(root_hash)? {
            root_node = n;
        } else {
            return Ok(leaf_map);
        }

        let mut cell_queue = VecDeque::with_capacity(keys.len());

        let root_cell =
            TreeCell::new::<BranchType, LeafType, DataType>(*root_hash, keys, root_node, 0);

        cell_queue.push_front(root_cell);

        while let Some(tree_cell) = cell_queue.pop_front() {
            if tree_cell.depth > self.depth {
                return Err(Exception::new("Depth of merkle tree exceeded"));
            }

            let node = tree_cell.node;

            match node.get_variant() {
                NodeVariant::Branch(branch) => {
                    let (_, zero, one, branch_split_index, branch_key) = branch.deconstruct();
                    let min_split_index = calc_min_split_index(tree_cell.keys, &branch_key);
                    let descendants = check_descendants(
                        tree_cell.keys,
                        branch_split_index,
                        &branch_key,
                        min_split_index,
                    );
                    if descendants.is_empty() {
                        continue;
                    }

                    let (zeros, ones) = split_pairs(descendants, branch_split_index);

                    if let Some(one_node) = self.db.get_node(&one)? {
                        if !ones.is_empty() {
                            let new_cell = TreeCell::new::<BranchType, LeafType, DataType>(
                                one,
                                ones,
                                one_node,
                                tree_cell.depth + 1,
                            );
                            cell_queue.push_front(new_cell);
                        }
                    }

                    if let Some(zero_node) = self.db.get_node(&zero)? {
                        if !zeros.is_empty() {
                            let new_cell = TreeCell::new::<BranchType, LeafType, DataType>(
                                zero,
                                zeros,
                                zero_node,
                                tree_cell.depth + 1,
                            );
                            cell_queue.push_front(new_cell);
                        }
                    }
                }
                NodeVariant::Leaf(n) => {
                    if let Some(d) = self.db.get_node(n.get_data())? {
                        if let NodeVariant::Data(data) = d.get_variant() {
                            let value = ValueType::decode(data.get_value())?;
                            if let Ok(index) = keys.binary_search(&n.get_key()) {
                                leaf_map.insert(keys[index], Some(value));
                            }
                        } else {
                            return Err(Exception::new(
                                "Corrupt merkle tree: Found non data node after leaf",
                            ));
                        }
                    } else {
                        return Err(Exception::new(
                            "Corrupt merkle tree: Failed to get leaf node from DB",
                        ));
                    }
                }
                NodeVariant::Data(_) => {
                    return Err(Exception::new(
                        "Corrupt merkle tree: Found data node while traversing tree",
                    ));
                }
            }
        }

        Ok(leaf_map)
    }

    /// Insert items into the `MerkleBIT`.  Keys must be sorted.  Returns a new root hash for the `MerkleBIT`.
    #[inline]
    pub fn insert(
        &mut self,
        previous_root: Option<&[u8; KEY_LEN]>,
        keys: &mut [&[u8; KEY_LEN]],
        values: &mut [&ValueType],
    ) -> BinaryMerkleTreeResult<[u8; KEY_LEN]> {
        if keys.len() != values.len() {
            return Err(Exception::new("Keys and values have different lengths"));
        }

        if keys.is_empty() || values.is_empty() {
            return Err(Exception::new("Keys or values are empty"));
        }

        let mut value_map = HashMap::new();
        for (&key, &value) in keys.iter().zip(values.iter()) {
            value_map.insert(key, value);
        }

        keys.sort();

        let nodes = self.insert_leaves(keys, &value_map)?;

        let mut tree_refs = Vec::with_capacity(keys.len());
        let mut key_map = HashMap::new();
        for (loc, &&key) in nodes.into_iter().zip(keys.iter()) {
            key_map.insert(key, loc);
            let tree_ref = TreeRef::new(key, loc, 1, 1);
            tree_refs.push(tree_ref);
        }

        if let Some(root) = previous_root {
            let mut proof_nodes = self.generate_treerefs(root, keys, &key_map)?;
            tree_refs.append(&mut proof_nodes);
        }

        let new_root = self.create_tree(tree_refs)?;
        Ok(new_root)
    }

    /// Traverses the tree and searches for nodes to include in the merkle proof.
    fn generate_treerefs(
        &mut self,
        root: &[u8; KEY_LEN],
        keys: &mut [&[u8; KEY_LEN]],
        key_map: &HashMap<[u8; KEY_LEN], [u8; KEY_LEN]>,
    ) -> BinaryMerkleTreeResult<Vec<TreeRef>> {
        // Nodes that form the merkle proof for the new tree
        let mut proof_nodes = Vec::with_capacity(keys.len());

        let root_node = if let Some(m) = self.db.get_node(root)? {
            m
        } else {
            return Err(Exception::new("Could not find root"));
        };

        let mut cell_queue = VecDeque::with_capacity(keys.len());
        let root_cell: TreeCell<NodeType> =
            TreeCell::new::<BranchType, LeafType, DataType>(*root, keys, root_node, 0);
        cell_queue.push_front(root_cell);

        while !cell_queue.is_empty() {
            let tree_cell = cell_queue
                .pop_front()
                .expect("cell queue should not be empty");

            if tree_cell.depth > self.depth {
                return Err(Exception::new("Depth of merkle tree exceeded"));
            }

            let node = tree_cell.node;

            let branch;
            let mut refs = node.get_references();
            match node.get_variant() {
                NodeVariant::Branch(n) => branch = n,
                NodeVariant::Leaf(n) => {
                    let key = n.get_key();

                    let mut update = false;

                    // Check if we are updating an existing value
                    if let Some(loc) = key_map.get(key) {
                        update = loc == &tree_cell.location;
                        if !update {
                            continue;
                        }
                    }

                    if let Some(mut l) = self.db.get_node(&tree_cell.location)? {
                        let leaf_refs = l.get_references() + 1;
                        l.set_references(leaf_refs);
                        self.db.insert(tree_cell.location, l)?;
                    } else {
                        return Err(Exception::new(
                            "Corrupt merkle tree: Failed to update leaf references",
                        ));
                    }

                    if update {
                        continue;
                    }

                    let tree_ref = TreeRef::new(*key, tree_cell.location, 1, 1);
                    proof_nodes.push(tree_ref);
                    continue;
                }
                NodeVariant::Data(_) => {
                    return Err(Exception::new(
                        "Corrupt merkle tree: Found data node while traversing tree",
                    ))
                }
            }

            let (branch_count, branch_zero, branch_one, branch_split_index, branch_key) =
                branch.deconstruct();

            let min_split_index = calc_min_split_index(tree_cell.keys, &branch_key);

            let mut descendants = tree_cell.keys;

            if min_split_index < branch_split_index {
                descendants = check_descendants(
                    tree_cell.keys,
                    branch_split_index,
                    &branch_key,
                    min_split_index,
                );

                if descendants.is_empty() {
                    let mut new_branch = BranchType::new();
                    new_branch.set_count(branch_count);
                    new_branch.set_zero(branch_zero);
                    new_branch.set_one(branch_one);
                    new_branch.set_split_index(branch_split_index);
                    new_branch.set_key(branch_key);

                    let tree_ref = TreeRef::new(branch_key, tree_cell.location, branch_count, 1);
                    refs += 1;
                    let mut new_node = NodeType::new(NodeVariant::Branch(new_branch));
                    new_node.set_references(refs);
                    self.db.insert(tree_ref.location, new_node)?;
                    proof_nodes.push(tree_ref);
                    continue;
                }
            }

            let (zeros, ones) = split_pairs(descendants, branch_split_index);
            if let Some(one_node) = self.db.get_node(&branch_one)? {
                if ones.is_empty() {
                    let other_key;
                    let count;
                    let one_refs = one_node.get_references() + 1;
                    let mut new_one_node;
                    match one_node.get_variant() {
                        NodeVariant::Branch(b) => {
                            count = b.get_count();
                            other_key = *b.get_key();
                            new_one_node = NodeType::new(NodeVariant::Branch(b));
                        }
                        NodeVariant::Leaf(l) => {
                            count = 1;
                            other_key = *l.get_key();
                            new_one_node = NodeType::new(NodeVariant::Leaf(l));
                        }
                        NodeVariant::Data(_) => {
                            return Err(Exception::new(
                                "Corrupt merkle tree: Found data node while traversing tree",
                            ));
                        }
                    }
                    new_one_node.set_references(one_refs);
                    self.db.insert(branch_one, new_one_node)?;
                    let tree_ref = TreeRef::new(other_key, branch_one, count, 1);
                    proof_nodes.push(tree_ref);
                } else {
                    let new_cell = TreeCell::new::<BranchType, LeafType, DataType>(
                        branch_one,
                        ones,
                        one_node,
                        tree_cell.depth + 1,
                    );
                    cell_queue.push_front(new_cell);
                }
            }
            if let Some(zero_node) = self.db.get_node(&branch_zero)? {
                if zeros.is_empty() {
                    let other_key;
                    let count;
                    let zero_refs = zero_node.get_references() + 1;
                    let mut new_zero_node;
                    match zero_node.get_variant() {
                        NodeVariant::Branch(b) => {
                            count = b.get_count();
                            other_key = *b.get_key();
                            new_zero_node = NodeType::new(NodeVariant::Branch(b));
                        }
                        NodeVariant::Leaf(l) => {
                            count = 1;
                            other_key = *l.get_key();
                            new_zero_node = NodeType::new(NodeVariant::Leaf(l));
                        }
                        NodeVariant::Data(_) => {
                            return Err(Exception::new(
                                "Corrupt merkle tree: Found data node while traversing tree",
                            ));
                        }
                    }
                    new_zero_node.set_references(zero_refs);
                    self.db.insert(branch_zero, new_zero_node)?;
                    let tree_ref = TreeRef::new(other_key, branch_zero, count, 1);
                    proof_nodes.push(tree_ref);
                } else {
                    let new_cell = TreeCell::new::<BranchType, LeafType, DataType>(
                        branch_zero,
                        zeros,
                        zero_node,
                        tree_cell.depth + 1,
                    );
                    cell_queue.push_front(new_cell);
                }
            }
        }

        Ok(proof_nodes)
    }

    /// Inserts all the new leaves into the database.
    /// Updates reference count if a leaf already exists.
    fn insert_leaves(
        &mut self,
        keys: &[&[u8; KEY_LEN]],
        values: &HashMap<&[u8; KEY_LEN], &ValueType>,
    ) -> BinaryMerkleTreeResult<Vec<[u8; KEY_LEN]>> {
        let mut nodes = Vec::with_capacity(keys.len());
        for &key in keys.iter() {
            // Create data node
            let mut data = DataType::new();
            data.set_value(&values[key].encode()?);

            let mut data_hasher = HasherType::new(KEY_LEN);
            data_hasher.update(b"d");
            data_hasher.update(key);
            data_hasher.update(data.get_value());
            let data_node_location = data_hasher.finalize();

            let mut data_node = NodeType::new(NodeVariant::Data(data));
            data_node.set_references(1);

            // Create leaf node
            let mut leaf = LeafType::new();
            leaf.set_data(data_node_location);
            leaf.set_key(*key);

            let mut leaf_hasher = HasherType::new(KEY_LEN);
            leaf_hasher.update(b"l");
            leaf_hasher.update(key);
            leaf_hasher.update(&leaf.get_data()[..]);
            let leaf_node_location = leaf_hasher.finalize();

            let mut leaf_node = NodeType::new(NodeVariant::Leaf(leaf));
            leaf_node.set_references(1);

            if let Some(n) = self.db.get_node(&data_node_location)? {
                let references = n.get_references() + 1;
                data_node.set_references(references);
            }

            if let Some(n) = self.db.get_node(&leaf_node_location)? {
                let references = n.get_references() + 1;
                leaf_node.set_references(references);
            }

            self.db.insert(data_node_location, data_node)?;
            self.db.insert(leaf_node_location, leaf_node)?;

            nodes.push(leaf_node_location);
        }
        Ok(nodes)
    }

    /// This function generates the queue of `TreeRef`s and merges the queue together to create a
    /// new tree root.
    fn create_tree(
        &mut self,
        mut tree_refs: Vec<TreeRef>,
    ) -> BinaryMerkleTreeResult<[u8; KEY_LEN]> {
        assert!(!tree_refs.is_empty());

        if tree_refs.len() == 1 {
            self.db.batch_write()?;
            let node = tree_refs.remove(0);
            return Ok(node.location);
        }

        tree_refs.sort();

        let mut tree_ref_queue = HashMap::new();

        let (tree_refs_raw, unique_split_bits) =
            Self::generate_tree_ref_queue(&mut tree_refs, &mut tree_ref_queue)?;
        let mut indices = unique_split_bits.into_iter().collect::<Vec<_>>();
        indices.sort();

        let mut root = None;
        for i in indices.into_iter().rev() {
            let level = tree_ref_queue
                .remove(&i)
                .expect("Level should not be empty");
            root = self.merge_nodes(tree_refs_raw, level)?;
        }
        Ok(root.expect("Failed to get root"))
    }

    /// Performs the merging of `TreeRef`s until a single new root is left.
    fn merge_nodes(
        &mut self,
        tree_refs_raw: *mut TreeRef,
        level: Vec<(u8, *mut TreeRef, *mut TreeRef, usize)>,
    ) -> BinaryMerkleTreeResult<Option<[u8; KEY_LEN]>> {
        let mut root = [0; 32];
        for (split_index, tree_ref_pointer, next_tree_ref_pointer, index) in level {
            let mut branch = BranchType::new();

            let tree_ref_key = unsafe { (*tree_ref_pointer).key };
            let tree_ref_location = unsafe { (*tree_ref_pointer).location };
            let tree_ref_count = unsafe { (*tree_ref_pointer).node_count };

            // Find the rightmost edge of the adjacent subtree
            let mut lookahead_count;
            let mut lookahead_tree_ref_pointer;
            unsafe {
                let mut count_ = (*next_tree_ref_pointer).count;

                if count_ > 1 {
                    // Look ahead by the count from our position
                    lookahead_tree_ref_pointer = tree_refs_raw.add(index + count_ as usize);
                    lookahead_count = (*lookahead_tree_ref_pointer).count;
                    while lookahead_count > count_ {
                        count_ = lookahead_count;
                        lookahead_tree_ref_pointer = tree_refs_raw.add(index + count_ as usize);
                        lookahead_count = (*lookahead_tree_ref_pointer).count;
                    }
                } else {
                    lookahead_count = count_;
                    lookahead_tree_ref_pointer = next_tree_ref_pointer;
                }
            }

            let next_tree_ref_location = unsafe { (*lookahead_tree_ref_pointer).location };
            let count = unsafe { tree_ref_count + (*lookahead_tree_ref_pointer).node_count };
            let branch_node_location;
            {
                let mut branch_hasher = HasherType::new(KEY_LEN);
                branch_hasher.update(b"b");
                branch_hasher.update(&tree_ref_location[..]);
                branch_hasher.update(&next_tree_ref_location[..]);
                branch_node_location = branch_hasher.finalize();

                branch.set_zero(tree_ref_location);
                branch.set_one(next_tree_ref_location);
                branch.set_count(count);
                branch.set_split_index(split_index);
                branch.set_key(tree_ref_key);
            }

            let mut branch_node = NodeType::new(NodeVariant::Branch(branch));
            branch_node.set_references(1);

            self.db.insert(branch_node_location, branch_node)?;

            unsafe {
                (*lookahead_tree_ref_pointer).key = tree_ref_key;
                (*lookahead_tree_ref_pointer).location = branch_node_location;
                (*lookahead_tree_ref_pointer).count = lookahead_count + (*tree_ref_pointer).count;
                (*lookahead_tree_ref_pointer).node_count = count;
                let tree_rcs_raw_access = tree_refs_raw.add(index);
                *tree_rcs_raw_access = *lookahead_tree_ref_pointer;
            }

            root = branch_node_location;
        }
        Ok(Some(root))
    }

    /// Generates the `TreeRef`s that will be made into the new tree.
    fn generate_tree_ref_queue(
        tree_refs: &mut Vec<TreeRef>,
        tree_ref_queue: &mut HashMap<u8, Vec<(u8, *mut TreeRef, *mut TreeRef, usize)>>,
    ) -> BinaryMerkleTreeResult<(*mut TreeRef, HashSet<u8>)> {
        let tree_rcs_raw = tree_refs.as_mut_ptr();
        let mut unique_split_bits = HashSet::new();
        for i in 0..tree_refs.len() - 1 {
            let left_key = tree_refs[i].key;
            let right_key = tree_refs[i + 1].key;

            for j in 0..KEY_LEN {
                if j == KEY_LEN - 1 && left_key[j] == right_key[j] {
                    // The keys are the same and don't diverge
                    return Err(Exception::new(
                        "Attempted to insert item with duplicate keys",
                    ));
                }
                // Skip bytes until we find a difference
                if left_key[j] == right_key[j] {
                    continue;
                }

                // Find the bit index of the first difference
                let xor_key = left_key[j] ^ right_key[j];
                let split_bit = (j * 8) as u8 + (7 - fast_log_2(xor_key) as u8);
                unique_split_bits.insert(split_bit);
                let new_item =
                    unsafe { (split_bit, tree_rcs_raw.add(i), tree_rcs_raw.add(i + 1), i) };
                if let Some(v) = tree_ref_queue.get_mut(&split_bit) {
                    v.push(new_item);
                } else {
                    tree_ref_queue.insert(split_bit, vec![new_item]);
                }

                break;
            }
        }
        Ok((tree_rcs_raw, unique_split_bits))
    }

    /// Remove all items with less than 1 reference under the given root.
    #[inline]
    pub fn remove(&mut self, root_hash: &[u8; KEY_LEN]) -> BinaryMerkleTreeResult<()> {
        let mut nodes = VecDeque::with_capacity(128);
        nodes.push_front(*root_hash);

        while !nodes.is_empty() {
            let node_location = if let Some(l) = nodes.pop_front() {
                l
            } else {
                return Err(Exception::new("Empty node queue"));
            };

            let mut node;
            if let Some(n) = self.db.get_node(&node_location)? {
                node = n;
            } else {
                continue;
            }

            let mut refs = node.get_references();
            if refs > 0 {
                refs -= 1;
            }

            let mut new_node;
            match node.get_variant() {
                NodeVariant::Branch(b) => {
                    if refs == 0 {
                        let zero = *b.get_zero();
                        let one = *b.get_one();
                        nodes.push_back(zero);
                        nodes.push_back(one);
                        self.db.remove(&node_location)?;
                        continue;
                    }
                    new_node = NodeType::new(NodeVariant::Branch(b))
                }
                NodeVariant::Leaf(l) => {
                    if refs == 0 {
                        let data = *l.get_data();
                        nodes.push_back(data);
                        self.db.remove(&node_location)?;
                        continue;
                    }
                    new_node = NodeType::new(NodeVariant::Leaf(l));
                }
                NodeVariant::Data(d) => {
                    if refs == 0 {
                        self.db.remove(&node_location)?;
                        continue;
                    }
                    new_node = NodeType::new(NodeVariant::Data(d))
                }
            }

            new_node.set_references(refs);
            self.db.insert(node_location, new_node)?;
        }
        self.db.batch_write()?;

        Ok(())
    }
}

#[cfg(feature = "use_rayon")]
impl<DatabaseType, BranchType, LeafType, DataType, NodeType, HasherType, ValueType>
    MerkleBIT<DatabaseType, BranchType, LeafType, DataType, NodeType, HasherType, ValueType>
where
    DatabaseType: Database<NodeType = NodeType> + Send + Sync,
    BranchType: Branch,
    LeafType: Leaf,
    DataType: Data,
    NodeType: Node<BranchType, LeafType, DataType> + Send + Sync,
    HasherType: Hasher<HashType = HasherType>,
    ValueType: Decode + Encode + Sync + Send,
{
    /// Create a new MerkleBIT from a saved database
    #[inline]
    pub fn new(path: &PathBuf, depth: usize) -> BinaryMerkleTreeResult<Self> {
        let db = DatabaseType::open(path)?;
        Ok(Self {
            db,
            depth,
            branch: PhantomData,
            leaf: PhantomData,
            data: PhantomData,
            node: PhantomData,
            hasher: PhantomData,
            value: PhantomData,
        })
    }

    /// Create a new MerkleBIT from an already opened database
    #[inline]
    pub fn from_db(db: DatabaseType, depth: usize) -> BinaryMerkleTreeResult<Self> {
        Ok(Self {
            db,
            depth,
            branch: PhantomData,
            leaf: PhantomData,
            data: PhantomData,
            node: PhantomData,
            hasher: PhantomData,
            value: PhantomData,
        })
    }

    /// Get items from the MerkleBIT.  Returns a map of Options which may include the corresponding values.
    #[inline]
    pub fn get<'a>(
        &self,
        root_hash: &[u8; KEY_LEN],
        keys: &mut [&'a [u8; KEY_LEN]],
    ) -> BinaryMerkleTreeResult<HashMap<&'a [u8; KEY_LEN], Option<ValueType>>> {
        if keys.is_empty() {
            return Ok(HashMap::new());
        }

        let mut leaf_map = generate_leaf_map(keys);

        #[cfg(not(feature = "use_rayon"))]
        keys.sort();
        #[cfg(feature = "use_rayon")]
        keys.par_sort();

        let root_node;
        if let Some(n) = self.db.get_node(root_hash)? {
            root_node = n;
        } else {
            return Ok(leaf_map);
        }

        let mut cell_queue = VecDeque::with_capacity(keys.len());

        let root_cell =
            TreeCell::new::<BranchType, LeafType, DataType>(*root_hash, keys, root_node, 0);

        cell_queue.push_front(root_cell);

        while let Some(tree_cell) = cell_queue.pop_front() {
            if tree_cell.depth > self.depth {
                return Err(Exception::new("Depth of merkle tree exceeded"));
            }

            let node = tree_cell.node;

            match node.get_variant() {
                NodeVariant::Branch(branch) => {
                    let (_, zero, one, branch_split_index, branch_key) = branch.deconstruct();
                    let min_split_index = calc_min_split_index(tree_cell.keys, &branch_key);
                    let descendants = check_descendants(
                        tree_cell.keys,
                        branch_split_index,
                        &branch_key,
                        min_split_index,
                    );
                    if descendants.is_empty() {
                        continue;
                    }

                    let (zeros, ones) = split_pairs(descendants, branch_split_index);

                    if let Some(one_node) = self.db.get_node(&one)? {
                        if !ones.is_empty() {
                            let new_cell = TreeCell::new::<BranchType, LeafType, DataType>(
                                one,
                                ones,
                                one_node,
                                tree_cell.depth + 1,
                            );
                            cell_queue.push_front(new_cell);
                        }
                    }

                    if let Some(zero_node) = self.db.get_node(&zero)? {
                        if !zeros.is_empty() {
                            let new_cell = TreeCell::new::<BranchType, LeafType, DataType>(
                                zero,
                                zeros,
                                zero_node,
                                tree_cell.depth + 1,
                            );
                            cell_queue.push_front(new_cell);
                        }
                    }
                }
                NodeVariant::Leaf(n) => {
                    if let Some(d) = self.db.get_node(n.get_data())? {
                        if let NodeVariant::Data(data) = d.get_variant() {
                            let value = ValueType::decode(data.get_value())?;
                            if let Ok(index) = keys.binary_search(&n.get_key()) {
                                leaf_map.insert(keys[index], Some(value));
                            }
                        } else {
                            return Err(Exception::new(
                                "Corrupt merkle tree: Found non data node after leaf",
                            ));
                        }
                    } else {
                        return Err(Exception::new(
                            "Corrupt merkle tree: Failed to get leaf node from DB",
                        ));
                    }
                }
                NodeVariant::Data(_) => {
                    return Err(Exception::new(
                        "Corrupt merkle tree: Found data node while traversing tree",
                    ));
                }
            }
        }

        Ok(leaf_map)
    }

    /// Insert items into the MerkleBIT.  Keys must be sorted.  Returns a new root hash for the MerkleBIT.
    #[inline]
    pub fn insert(
        &mut self,
        previous_root: Option<&[u8; KEY_LEN]>,
        keys: &mut [&[u8; KEY_LEN]],
        values: &mut [&ValueType],
    ) -> BinaryMerkleTreeResult<[u8; KEY_LEN]> {
        if keys.len() != values.len() {
            return Err(Exception::new("Keys and values have different lengths"));
        }

        if keys.is_empty() || values.is_empty() {
            return Err(Exception::new("Keys or values are empty"));
        }

        let mut value_map = HashMap::new();
        for (&key, &value) in keys.iter().zip(values.iter()) {
            value_map.insert(key, value);
        }

        keys.par_sort();

        let nodes = self.insert_leaves(keys, &value_map)?;

        let mut tree_refs = Vec::with_capacity(keys.len());
        let mut key_map = HashMap::new();
        for (loc, &&key) in nodes.into_iter().zip(keys.iter()) {
            key_map.insert(key, loc);
            let tree_ref = TreeRef::new(key, loc, 1, 1);
            tree_refs.push(tree_ref);
        }

        if let Some(root) = previous_root {
            let mut proof_nodes = self.generate_treerefs(root, keys, &key_map)?;
            tree_refs.append(&mut proof_nodes);
        }

        let new_root = self.create_tree(tree_refs)?;
        Ok(new_root)
    }

    /// Traverses the tree and searches for nodes to include in the merkle proof.
    fn generate_treerefs(
        &mut self,
        root: &[u8; KEY_LEN],
        keys: &mut [&[u8; KEY_LEN]],
        key_map: &HashMap<[u8; KEY_LEN], [u8; KEY_LEN]>,
    ) -> BinaryMerkleTreeResult<Vec<TreeRef>> {
        // Nodes that form the merkle proof for the new tree
        let mut proof_nodes = Vec::with_capacity(keys.len());

        let root_node = if let Some(m) = self.db.get_node(root)? {
            m
        } else {
            return Err(Exception::new("Could not find root"));
        };

        let mut cell_queue = VecDeque::with_capacity(keys.len());
        let root_cell: TreeCell<NodeType> =
            TreeCell::new::<BranchType, LeafType, DataType>(*root, keys, root_node, 0);
        cell_queue.push_front(root_cell);

        while !cell_queue.is_empty() {
            let tree_cell = cell_queue
                .pop_front()
                .expect("cell queue should not be empty");

            if tree_cell.depth > self.depth {
                return Err(Exception::new("Depth of merkle tree exceeded"));
            }

            let node = tree_cell.node;

            let branch;
            let mut refs = node.get_references();
            match node.get_variant() {
                NodeVariant::Branch(n) => branch = n,
                NodeVariant::Leaf(n) => {
                    let key = n.get_key();

                    let mut update = false;

                    // Check if we are updating an existing value
                    if let Some(loc) = key_map.get(key) {
                        update = loc == &tree_cell.location;
                        if !update {
                            continue;
                        }
                    }

                    if let Some(mut l) = self.db.get_node(&tree_cell.location)? {
                        let leaf_refs = l.get_references() + 1;
                        l.set_references(leaf_refs);
                        self.db.insert(tree_cell.location, l)?;
                    } else {
                        return Err(Exception::new(
                            "Corrupt merkle tree: Failed to update leaf references",
                        ));
                    }

                    if update {
                        continue;
                    }

                    let tree_ref = TreeRef::new(*key, tree_cell.location, 1, 1);
                    proof_nodes.push(tree_ref);
                    continue;
                }
                NodeVariant::Data(_) => {
                    return Err(Exception::new(
                        "Corrupt merkle tree: Found data node while traversing tree",
                    ))
                }
            }

            let (branch_count, branch_zero, branch_one, branch_split_index, branch_key) =
                branch.deconstruct();

            let min_split_index = calc_min_split_index(tree_cell.keys, &branch_key);

            let mut descendants = tree_cell.keys;

            if min_split_index < branch_split_index {
                descendants = check_descendants(
                    tree_cell.keys,
                    branch_split_index,
                    &branch_key,
                    min_split_index,
                );

                if descendants.is_empty() {
                    let mut new_branch = BranchType::new();
                    new_branch.set_count(branch_count);
                    new_branch.set_zero(branch_zero);
                    new_branch.set_one(branch_one);
                    new_branch.set_split_index(branch_split_index);
                    new_branch.set_key(branch_key);

                    let tree_ref = TreeRef::new(branch_key, tree_cell.location, branch_count, 1);
                    refs += 1;
                    let mut new_node = NodeType::new(NodeVariant::Branch(new_branch));
                    new_node.set_references(refs);
                    self.db.insert(tree_ref.location, new_node)?;
                    proof_nodes.push(tree_ref);
                    continue;
                }
            }

            let (zeros, ones) = split_pairs(descendants, branch_split_index);
            if let Some(one_node) = self.db.get_node(&branch_one)? {
                if ones.is_empty() {
                    let other_key;
                    let count;
                    let one_refs = one_node.get_references() + 1;
                    let mut new_one_node;
                    match one_node.get_variant() {
                        NodeVariant::Branch(b) => {
                            count = b.get_count();
                            other_key = *b.get_key();
                            new_one_node = NodeType::new(NodeVariant::Branch(b));
                        }
                        NodeVariant::Leaf(l) => {
                            count = 1;
                            other_key = *l.get_key();
                            new_one_node = NodeType::new(NodeVariant::Leaf(l));
                        }
                        NodeVariant::Data(_) => {
                            return Err(Exception::new(
                                "Corrupt merkle tree: Found data node while traversing tree",
                            ));
                        }
                    }
                    new_one_node.set_references(one_refs);
                    self.db.insert(branch_one, new_one_node)?;
                    let tree_ref = TreeRef::new(other_key, branch_one, count, 1);
                    proof_nodes.push(tree_ref);
                } else {
                    let new_cell = TreeCell::new::<BranchType, LeafType, DataType>(
                        branch_one,
                        ones,
                        one_node,
                        tree_cell.depth + 1,
                    );
                    cell_queue.push_front(new_cell);
                }
            }
            if let Some(zero_node) = self.db.get_node(&branch_zero)? {
                if zeros.is_empty() {
                    let other_key;
                    let count;
                    let zero_refs = zero_node.get_references() + 1;
                    let mut new_zero_node;
                    match zero_node.get_variant() {
                        NodeVariant::Branch(b) => {
                            count = b.get_count();
                            other_key = *b.get_key();
                            new_zero_node = NodeType::new(NodeVariant::Branch(b));
                        }
                        NodeVariant::Leaf(l) => {
                            count = 1;
                            other_key = *l.get_key();
                            new_zero_node = NodeType::new(NodeVariant::Leaf(l));
                        }
                        NodeVariant::Data(_) => {
                            return Err(Exception::new(
                                "Corrupt merkle tree: Found data node while traversing tree",
                            ));
                        }
                    }
                    new_zero_node.set_references(zero_refs);
                    self.db.insert(branch_zero, new_zero_node)?;
                    let tree_ref = TreeRef::new(other_key, branch_zero, count, 1);
                    proof_nodes.push(tree_ref);
                } else {
                    let new_cell = TreeCell::new::<BranchType, LeafType, DataType>(
                        branch_zero,
                        zeros,
                        zero_node,
                        tree_cell.depth + 1,
                    );
                    cell_queue.push_front(new_cell);
                }
            }
        }

        Ok(proof_nodes)
    }

    /// Inserts all the new leaves into the database.
    /// Updates reference count if a leaf already exists.
    fn insert_leaves(
        &mut self,
        keys: &[&[u8; KEY_LEN]],
        values: &HashMap<&[u8; KEY_LEN], &ValueType>,
    ) -> BinaryMerkleTreeResult<Vec<[u8; KEY_LEN]>> {
        let db = &self.db;

        let nodes = keys
            .par_iter()
            .map(|&key| {
                let mut data = DataType::new();
                data.set_value(&values[key].encode().expect("Error encoding value"));

                let mut data_hasher = HasherType::new(KEY_LEN);
                data_hasher.update(b"d");
                data_hasher.update(key);
                data_hasher.update(data.get_value());
                let data_node_location = data_hasher.finalize();

                let mut data_node = NodeType::new(NodeVariant::Data(data));
                data_node.set_references(1);

                // Create leaf node
                let mut leaf = LeafType::new();
                leaf.set_data(data_node_location);
                leaf.set_key(*key);

                let mut leaf_hasher = HasherType::new(KEY_LEN);
                leaf_hasher.update(b"l");
                leaf_hasher.update(key);
                leaf_hasher.update(&leaf.get_data()[..]);
                let leaf_node_location = leaf_hasher.finalize();

                let mut leaf_node = NodeType::new(NodeVariant::Leaf(leaf));
                leaf_node.set_references(1);

                if let Some(n) = db
                    .get_node(&data_node_location)
                    .expect("Error loading data node")
                {
                    let references = n.get_references() + 1;
                    data_node.set_references(references);
                }

                if let Some(n) = db
                    .get_node(&leaf_node_location)
                    .expect("Error loading leaf node")
                {
                    let references = n.get_references() + 1;
                    leaf_node.set_references(references);
                }

                (data_node_location, data_node, leaf_node_location, leaf_node)
            })
            .collect::<Vec<_>>();

        let node_locations = nodes
            .into_iter()
            .map(|x| {
                self.db.insert(x.0, x.1).expect("Failed to insert into db");
                self.db.insert(x.2, x.3).expect("Failed to insert into db");
                x.2
            })
            .collect::<Vec<_>>();

        Ok(node_locations)
    }

    /// This function generates the queue of `TreeRef`s and merges the queue together to create a
    /// new tree root.
    fn create_tree(
        &mut self,
        mut tree_refs: Vec<TreeRef>,
    ) -> BinaryMerkleTreeResult<[u8; KEY_LEN]> {
        assert!(!tree_refs.is_empty());

        if tree_refs.len() == 1 {
            self.db.batch_write()?;
            let node = tree_refs.remove(0);
            return Ok(node.location);
        }

        tree_refs.sort();

        let mut tree_ref_queue = HashMap::new();

        let (tree_refs_raw, unique_split_bits) =
            Self::generate_tree_ref_queue(&mut tree_refs, &mut tree_ref_queue)?;
        let mut indices = unique_split_bits.into_iter().collect::<Vec<_>>();
        indices.sort();

        let mut root = [0; 32];
        for i in indices.into_iter().rev() {
            let level = tree_ref_queue
                .remove(&i)
                .expect("Level should not be empty");
            root = self.merge_nodes(tree_refs_raw, level)?;
        }
        self.db.batch_write()?;
        Ok(root)
    }

    /// Performs the merging of `TreeRef`s until a single new root is left.
    fn merge_nodes(
        &mut self,
        tree_rcs_raw: *mut TreeRef,
        level: Vec<MergeCell>,
    ) -> BinaryMerkleTreeResult<[u8; KEY_LEN]> {
        let raw_tree_refs = TreeRefRaw(tree_rcs_raw);
        let root: Vec<BinaryMerkleTreeResult<([u8; 32], u8, NodeType)>> = level
            .into_par_iter()
            .map(|merge_cell| {
                let (split_index, tree_ref_pointer, next_tree_ref_pointer, index) =
                    merge_cell.deconstruct();
                let mut branch = BranchType::new();

                let tree_ref_key = unsafe { (*tree_ref_pointer).key };
                let tree_ref_location = unsafe { (*tree_ref_pointer).location };
                let tree_ref_count = unsafe { (*tree_ref_pointer).node_count };

                // Find the rightmost edge of the adjacent subtree
                let mut lookahead_count;
                let mut lookahead_tree_ref_pointer;
                unsafe {
                    let mut count_ = (*next_tree_ref_pointer).count;

                    if count_ > 1 {
                        // Look ahead by the count from our position
                        lookahead_tree_ref_pointer = raw_tree_refs.add(index + count_ as usize);
                        lookahead_count = (*lookahead_tree_ref_pointer).count;
                        while lookahead_count > count_ {
                            count_ = lookahead_count;
                            lookahead_tree_ref_pointer = raw_tree_refs.add(index + count_ as usize);
                            lookahead_count = (*lookahead_tree_ref_pointer).count;
                        }
                    } else {
                        lookahead_count = count_;
                        lookahead_tree_ref_pointer = next_tree_ref_pointer;
                    }
                }

                let next_tree_ref_location = unsafe { (*lookahead_tree_ref_pointer).location };
                let count = unsafe { tree_ref_count + (*lookahead_tree_ref_pointer).node_count };
                let branch_node_location;
                {
                    let mut branch_hasher = HasherType::new(KEY_LEN);
                    branch_hasher.update(b"b");
                    branch_hasher.update(&tree_ref_location[..]);
                    branch_hasher.update(&next_tree_ref_location[..]);
                    branch_node_location = branch_hasher.finalize();

                    branch.set_zero(tree_ref_location);
                    branch.set_one(next_tree_ref_location);
                    branch.set_count(count);
                    branch.set_split_index(split_index);
                    branch.set_key(tree_ref_key);
                }

                let mut branch_node = NodeType::new(NodeVariant::Branch(branch));
                branch_node.set_references(1);

                unsafe {
                    (*lookahead_tree_ref_pointer).key = tree_ref_key;
                    (*lookahead_tree_ref_pointer).location = branch_node_location;
                    (*lookahead_tree_ref_pointer).count =
                        lookahead_count + (*tree_ref_pointer).count;
                    (*lookahead_tree_ref_pointer).node_count = count;
                    let tree_rcs_raw_access = raw_tree_refs.add(index);
                    *tree_rcs_raw_access = *lookahead_tree_ref_pointer;
                }

                Ok((branch_node_location, split_index, branch_node))
            })
            .collect::<Vec<_>>();

        let root_len = root.len() - 1;
        for (i, result) in root.into_iter().enumerate() {
            if let Ok(item) = result {
                self.db.insert(item.0, item.2)?;
                if i == root_len {
                    return Ok(item.0);
                }
            } else {
                return Err(Exception::new("Failed to make tree root"));
            }
        }

        self.db.batch_write()?;

        Err(Exception::new("Failed to make tree root"))
    }

    /// Generates the `TreeRef`s that will be made into the new tree.
    fn generate_tree_ref_queue(
        tree_refs: &mut Vec<TreeRef>,
        tree_ref_queue: &mut HashMap<u8, Vec<MergeCell>>,
    ) -> BinaryMerkleTreeResult<(*mut TreeRef, HashSet<u8>)> {
        let tree_rcs_raw = tree_refs.as_mut_ptr();
        let mut unique_split_bits = HashSet::new();
        for i in 0..tree_refs.len() - 1 {
            let left_key = tree_refs[i].key;
            let right_key = tree_refs[i + 1].key;

            for j in 0..KEY_LEN {
                if j == KEY_LEN - 1 && left_key[j] == right_key[j] {
                    // The keys are the same and don't diverge
                    return Err(Exception::new(
                        "Attempted to insert item with duplicate keys",
                    ));
                }
                // Skip bytes until we find a difference
                if left_key[j] == right_key[j] {
                    continue;
                }

                // Find the bit index of the first difference
                let xor_key = left_key[j] ^ right_key[j];
                let split_bit = (j * 8) as u8 + (7 - fast_log_2(xor_key) as u8);
                unique_split_bits.insert(split_bit);
                let new_item = unsafe {
                    MergeCell::new(split_bit, tree_rcs_raw.add(i), tree_rcs_raw.add(i + 1), i)
                };
                if let Some(v) = tree_ref_queue.get_mut(&split_bit) {
                    v.push(new_item);
                } else {
                    tree_ref_queue.insert(split_bit, vec![new_item]);
                }

                break;
            }
        }
        Ok((tree_rcs_raw, unique_split_bits))
    }

    /// Remove all items with less than 1 reference under the given root.
    #[inline]
    pub fn remove(&mut self, root_hash: &[u8; KEY_LEN]) -> BinaryMerkleTreeResult<()> {
        let mut nodes = VecDeque::with_capacity(128);
        nodes.push_front(*root_hash);

        while !nodes.is_empty() {
            let node_location = if let Some(l) = nodes.pop_front() {
                l
            } else {
                return Err(Exception::new("Empty node queue"));
            };

            let mut node;
            if let Some(n) = self.db.get_node(&node_location)? {
                node = n;
            } else {
                continue;
            }

            let mut refs = node.get_references();
            if refs > 0 {
                refs -= 1;
            }

            let mut new_node;
            match node.get_variant() {
                NodeVariant::Branch(b) => {
                    if refs == 0 {
                        let zero = *b.get_zero();
                        let one = *b.get_one();
                        nodes.push_back(zero);
                        nodes.push_back(one);
                        self.db.remove(&node_location)?;
                        continue;
                    }
                    new_node = NodeType::new(NodeVariant::Branch(b))
                }
                NodeVariant::Leaf(l) => {
                    if refs == 0 {
                        let data = *l.get_data();
                        nodes.push_back(data);
                        self.db.remove(&node_location)?;
                        continue;
                    }
                    new_node = NodeType::new(NodeVariant::Leaf(l));
                }
                NodeVariant::Data(d) => {
                    if refs == 0 {
                        self.db.remove(&node_location)?;
                        continue;
                    }
                    new_node = NodeType::new(NodeVariant::Data(d))
                }
            }

            new_node.set_references(refs);
            self.db.insert(node_location, new_node)?;
        }
        self.db.batch_write()?;

        Ok(())
    }
}

#[cfg(test)]
pub mod tests {
    use crate::utils::tree_utils::choose_zero;

    use super::*;

    #[test]
    fn it_chooses_the_right_branch_easy() {
        let key = [0x0F; KEY_LEN];
        for i in 0..8 {
            let expected_branch;
            if i < 4 {
                expected_branch = true;
            } else {
                expected_branch = false;
            }
            let branch = choose_zero(&key, i);
            assert_eq!(branch, expected_branch);
        }
    }

    #[test]
    fn it_chooses_the_right_branch_medium() {
        let key = [0x55; KEY_LEN];
        for i in 0..8 {
            let expected_branch;
            if i % 2 == 0 {
                expected_branch = true;
            } else {
                expected_branch = false;
            }
            let branch = choose_zero(&key, i);
            assert_eq!(branch, expected_branch);
        }
        let key = [0xAA; KEY_LEN];
        for i in 0..8 {
            let expected_branch;
            if i % 2 == 0 {
                expected_branch = false;
            } else {
                expected_branch = true;
            }
            let branch = choose_zero(&key, i);
            assert_eq!(branch, expected_branch);
        }
    }

    #[test]
    fn it_chooses_the_right_branch_hard() {
        let key = [0x68; KEY_LEN];
        for i in 0..8 {
            let expected_branch;
            if i == 1 || i == 2 || i == 4 {
                expected_branch = false;
            } else {
                expected_branch = true;
            }
            let branch = choose_zero(&key, i);
            assert_eq!(branch, expected_branch);
        }

        let key = [0xAB; KEY_LEN];
        for i in 0..8 {
            let expected_branch;
            if i == 0 || i == 2 || i == 4 || i == 6 || i == 7 {
                expected_branch = false;
            } else {
                expected_branch = true;
            }
            let branch = choose_zero(&key, i);
            assert_eq!(branch, expected_branch);
        }
    }

    #[test]
    fn it_splits_an_all_zeros_sorted_list_of_pairs() {
        // The complexity of these tests result from the fact that getting a key and splitting the
        // tree should not require any copying or moving of memory.
        let zero_key = [0x00u8; KEY_LEN];
        let key_vec = vec![
            &zero_key, &zero_key, &zero_key, &zero_key, &zero_key, &zero_key, &zero_key, &zero_key,
            &zero_key, &zero_key,
        ];
        let keys = key_vec;

        let result = split_pairs(&keys, 0);
        assert_eq!(result.0.len(), 10);
        assert_eq!(result.1.len(), 0);
        for i in 0..result.0.len() {
            assert_eq!(*result.0[i], [0x00u8; KEY_LEN]);
        }
    }

    #[test]
    fn it_splits_an_all_ones_sorted_list_of_pairs() {
        let one_key = [0xFFu8; KEY_LEN];
        let keys = vec![
            &one_key, &one_key, &one_key, &one_key, &one_key, &one_key, &one_key, &one_key,
            &one_key, &one_key,
        ];
        let result = split_pairs(&keys, 0);
        assert_eq!(result.0.len(), 0);
        assert_eq!(result.1.len(), 10);
        for i in 0..result.1.len() {
            assert_eq!(*result.1[i], [0xFFu8; KEY_LEN]);
        }
    }

    #[test]
    fn it_splits_an_even_length_sorted_list_of_pairs() {
        let zero_key = [0x00u8; KEY_LEN];
        let one_key = [0xFFu8; KEY_LEN];
        let keys = vec![
            &zero_key, &zero_key, &zero_key, &zero_key, &zero_key, &one_key, &one_key, &one_key,
            &one_key, &one_key,
        ];
        let result = split_pairs(&keys, 0);
        assert_eq!(result.0.len(), 5);
        assert_eq!(result.1.len(), 5);
        for i in 0..result.0.len() {
            assert_eq!(*result.0[i], [0x00u8; KEY_LEN]);
        }
        for i in 0..result.1.len() {
            assert_eq!(*result.1[i], [0xFFu8; KEY_LEN]);
        }
    }

    #[test]
    fn it_splits_an_odd_length_sorted_list_of_pairs_with_more_zeros() {
        let zero_key = [0x00u8; KEY_LEN];
        let one_key = [0xFFu8; KEY_LEN];
        let keys = vec![
            &zero_key, &zero_key, &zero_key, &zero_key, &zero_key, &zero_key, &one_key, &one_key,
            &one_key, &one_key, &one_key,
        ];
        let result = split_pairs(&keys, 0);
        assert_eq!(result.0.len(), 6);
        assert_eq!(result.1.len(), 5);
        for i in 0..result.0.len() {
            assert_eq!(*result.0[i], [0x00u8; KEY_LEN]);
        }
        for i in 0..result.1.len() {
            assert_eq!(*result.1[i], [0xFFu8; KEY_LEN]);
        }
    }

    #[test]
    fn it_splits_an_odd_length_sorted_list_of_pairs_with_more_ones() {
        let zero_key = [0x00u8; KEY_LEN];
        let one_key = [0xFFu8; KEY_LEN];
        let keys = vec![
            &zero_key, &zero_key, &zero_key, &zero_key, &zero_key, &one_key, &one_key, &one_key,
            &one_key, &one_key, &one_key,
        ];

        let result = split_pairs(&keys, 0);
        assert_eq!(result.0.len(), 5);
        assert_eq!(result.1.len(), 6);
        for i in 0..result.0.len() {
            assert_eq!(*result.0[i], [0x00u8; KEY_LEN]);
        }
        for i in 0..result.1.len() {
            assert_eq!(*result.1[i], [0xFFu8; KEY_LEN]);
        }
    }
}

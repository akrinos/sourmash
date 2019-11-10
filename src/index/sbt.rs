use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::hash::{BuildHasherDefault, Hasher};
use std::io::{BufReader, Read};
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use derive_builder::Builder;
use failure::Error;
use lazy_init::Lazy;
use serde_derive::{Deserialize, Serialize};

use crate::index::nodegraph::Nodegraph;
use crate::index::storage::{FSStorage, ReadData, ReadDataError, Storage, StorageInfo};
use crate::index::{Comparable, Dataset, DatasetInfo, Index};
use crate::Signature;

pub type MHBT = SBT<Node<Nodegraph>, Dataset<Signature>>;

#[derive(Builder)]
pub struct SBT<N, L> {
    #[builder(default = "2")]
    d: u32,

    storage: Arc<dyn Storage>,

    #[builder(setter(skip))]
    factory: Factory,

    nodes: HashMap<u64, N>,

    leaves: HashMap<u64, L>,
}

const fn parent(pos: u64, d: u64) -> u64 {
    ((pos - 1) / d) as u64
}

const fn child(parent: u64, pos: u64, d: u64) -> u64 {
    d * parent + pos + 1
}

impl<N, L> SBT<N, L>
where
    L: std::clone::Clone,
{
    #[inline(always)]
    fn parent(&self, pos: u64) -> Option<u64> {
        if pos == 0 {
            None
        } else {
            Some(parent(pos, self.d as u64))
        }
    }

    #[inline(always)]
    fn child(&self, parent: u64, pos: u64) -> u64 {
        child(parent, pos, self.d as u64)
    }

    #[inline(always)]
    fn children(&self, pos: u64) -> Vec<u64> {
        (0..u64::from(self.d)).map(|c| self.child(pos, c)).collect()
    }

    pub fn leaves(&self) -> Vec<L> {
        self.leaves.values().cloned().collect()
    }

    pub fn storage(&self) -> Arc<dyn Storage> {
        Arc::clone(&self.storage)
    }

    // combine
}

impl<T, U> SBT<Node<U>, Dataset<T>>
where
    T: std::marker::Sync + Send,
    U: std::marker::Sync + Send,
{
    pub fn from_reader<R, P>(rdr: &mut R, path: P) -> Result<SBT<Node<U>, Dataset<T>>, Error>
    where
        R: Read,
        P: AsRef<Path>,
    {
        // TODO: check https://serde.rs/enum-representations.html for a
        // solution for loading v4 and v5
        let sbt: SBTInfo<NodeInfo, DatasetInfo> = serde_json::from_reader(rdr)?;

        // TODO: match with available Storage while we don't
        // add a function to build a Storage from a StorageInfo
        let mut basepath = PathBuf::new();
        basepath.push(path);
        basepath.push(&sbt.storage.args["path"]);

        let storage: Arc<dyn Storage> = Arc::new(FSStorage { basepath });

        Ok(SBT {
            d: sbt.d,
            factory: sbt.factory,
            storage: Arc::clone(&storage),
            nodes: sbt
                .nodes
                .into_iter()
                .map(|(n, l)| {
                    let new_node = Node {
                        filename: l.filename,
                        name: l.name,
                        metadata: l.metadata,
                        storage: Some(Arc::clone(&storage)),
                        data: Arc::new(Lazy::new()),
                    };
                    (n, new_node)
                })
                .collect(),
            leaves: sbt
                .leaves
                .into_iter()
                .map(|(n, l)| {
                    let new_node = Dataset {
                        filename: l.filename,
                        name: l.name,
                        metadata: l.metadata,
                        storage: Some(Arc::clone(&storage)),
                        data: Arc::new(Lazy::new()),
                    };
                    (n, new_node)
                })
                .collect(),
        })
    }

    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<SBT<Node<U>, Dataset<T>>, Error> {
        let file = File::open(&path)?;
        let mut reader = BufReader::new(file);

        // TODO: match with available Storage while we don't
        // add a function to build a Storage from a StorageInfo
        let mut basepath = PathBuf::new();
        basepath.push(path);
        basepath.canonicalize()?;

        let sbt =
            SBT::<Node<U>, Dataset<T>>::from_reader(&mut reader, &basepath.parent().unwrap())?;
        Ok(sbt)
    }

    pub fn save_file<P: AsRef<Path>>(&self, path: P) -> Result<(), Error> {
        let mut args: HashMap<String, String> = HashMap::default();
        args.insert("path".into(), ".".into());
        let storage = StorageInfo {
            backend: "FSStorage".into(),
            args: args,
        };
        let info: SBTInfo<NodeInfo, DatasetInfo> = SBTInfo {
            d: self.d,
            factory: self.factory.clone(),
            storage: storage,
            version: 5,
            nodes: self
                .nodes
                .iter()
                .map(|(n, l)| {
                    let new_node = NodeInfo {
                        filename: l.filename.clone(),
                        name: l.name.clone(),
                        metadata: l.metadata.clone(),
                    };
                    (*n, new_node)
                })
                .collect(),
            leaves: self
                .leaves
                .iter()
                .map(|(n, l)| {
                    let new_node = DatasetInfo {
                        filename: l.filename.clone(),
                        name: l.name.clone(),
                        metadata: l.metadata.clone(),
                    };
                    (*n, new_node)
                })
                .collect(),
        };
        let file = File::create(path)?;
        serde_json::to_writer(file, &info)?;

        Ok(())
    }
}

impl<N, L> Index for SBT<N, L>
where
    N: Comparable<N> + Comparable<L>,
    L: Comparable<L> + std::clone::Clone + std::fmt::Debug,
{
    type Item = L;

    fn find<F>(&self, search_fn: F, sig: &L, threshold: f64) -> Result<Vec<&L>, Error>
    where
        F: Fn(&dyn Comparable<Self::Item>, &Self::Item, f64) -> bool,
    {
        let mut matches = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = vec![0u64];

        while !queue.is_empty() {
            let pos = queue.pop().unwrap();
            if !visited.contains(&pos) {
                visited.insert(pos);

                if let Some(node) = self.nodes.get(&pos) {
                    if search_fn(&node, sig, threshold) {
                        for c in self.children(pos) {
                            queue.push(c);
                        }
                    }
                } else if let Some(leaf) = self.leaves.get(&pos) {
                    if search_fn(leaf, sig, threshold) {
                        matches.push(leaf);
                    }
                }
            }
        }

        Ok(matches)
    }

    fn insert(&mut self, dataset: &L) {}

    fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), Error> {
        Ok(())
    }

    fn load<P: AsRef<Path>>(path: P) -> Result<(), Error> {
        Ok(())
    }

    fn datasets(&self) -> Vec<Self::Item> {
        self.leaves.values().cloned().collect()
    }
}

#[derive(Builder, Clone, Default, Serialize, Deserialize)]
pub struct Factory {
    class: String,
    args: Vec<u64>,
}

#[derive(Builder, Default, Clone)]
pub struct Node<T>
where
    T: std::marker::Sync,
{
    filename: String,
    name: String,
    metadata: HashMap<String, u64>,
    storage: Option<Arc<dyn Storage>>,
    #[builder(setter(skip))]
    pub(crate) data: Arc<Lazy<T>>,
}

impl Comparable<Node<Nodegraph>> for Node<Nodegraph> {
    fn similarity(&self, other: &Node<Nodegraph>) -> f64 {
        let ng: &Nodegraph = self.data().unwrap();
        let ong: &Nodegraph = other.data().unwrap();
        ng.similarity(&ong)
    }

    fn containment(&self, other: &Node<Nodegraph>) -> f64 {
        let ng: &Nodegraph = self.data().unwrap();
        let ong: &Nodegraph = other.data().unwrap();
        ng.containment(&ong)
    }
}

impl Comparable<Dataset<Signature>> for Node<Nodegraph> {
    fn similarity(&self, other: &Dataset<Signature>) -> f64 {
        let ng: &Nodegraph = self.data().unwrap();
        let oth: &Signature = other.data().unwrap();

        // TODO: select the right signatures...
        let sig = &oth.signatures[0];
        if sig.size() == 0 {
            return 0.0;
        }

        let matches: usize = sig.mins.iter().map(|h| ng.get(*h)).sum();

        let min_n_below = self.metadata["min_n_below"] as f64;

        // This overestimates the similarity, but better than truncating too
        // soon and losing matches
        matches as f64 / min_n_below
    }

    fn containment(&self, other: &Dataset<Signature>) -> f64 {
        let ng: &Nodegraph = self.data().unwrap();
        let oth: &Signature = other.data().unwrap();

        // TODO: select the right signatures...
        let sig = &oth.signatures[0];
        if sig.size() == 0 {
            return 0.0;
        }

        let matches: usize = sig.mins.iter().map(|h| ng.get(*h)).sum();

        matches as f64 / sig.size() as f64
    }
}

impl ReadData<Nodegraph> for Node<Nodegraph> {
    fn data(&self) -> Result<&Nodegraph, Error> {
        if let Some(storage) = &self.storage {
            Ok(self.data.get_or_create(|| {
                let raw = storage.load(&self.filename).unwrap();
                Nodegraph::from_reader(&mut &raw[..]).unwrap()
            }))
        } else {
            Err(ReadDataError::LoadError.into())
        }
    }
}

#[derive(Serialize, Deserialize)]
struct NodeInfo {
    filename: String,
    name: String,
    metadata: HashMap<String, u64>,
}

#[derive(Serialize, Deserialize)]
struct SBTInfo<N, L> {
    d: u32,
    version: u32,
    storage: StorageInfo,
    factory: Factory,
    nodes: HashMap<u64, N>,
    leaves: HashMap<u64, L>,
}

// This comes from finch
pub struct NoHashHasher(u64);

impl Default for NoHashHasher {
    #[inline]
    fn default() -> NoHashHasher {
        NoHashHasher(0x0)
    }
}

impl Hasher for NoHashHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        *self = NoHashHasher(
            (u64::from(bytes[0]) << 24)
                + (u64::from(bytes[1]) << 16)
                + (u64::from(bytes[2]) << 8)
                + u64::from(bytes[3]),
        );
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

type HashIntersection = HashSet<u64, BuildHasherDefault<NoHashHasher>>;

enum BinaryTree {
    Empty,
    Internal(Box<TreeNode<HashIntersection>>),
    Dataset(Box<TreeNode<Dataset<Signature>>>),
}

struct TreeNode<T> {
    element: T,
    left: BinaryTree,
    right: BinaryTree,
}

pub fn scaffold<N>(mut datasets: Vec<Dataset<Signature>>) -> SBT<Node<N>, Dataset<Signature>>
where
    N: std::marker::Sync + std::clone::Clone + std::default::Default,
{
    let mut leaves: HashMap<u64, Dataset<Signature>> = HashMap::with_capacity(datasets.len());

    let mut next_round = Vec::new();

    // generate two bottom levels:
    // - datasets
    // - first level of internal nodes
    eprintln!("Start processing leaves");
    while !datasets.is_empty() {
        let next_leaf = datasets.pop().unwrap();

        let (simleaf_tree, in_common) = if datasets.is_empty() {
            (
                BinaryTree::Empty,
                HashIntersection::from_iter(next_leaf.mins().into_iter()),
            )
        } else {
            let mut similar_leaf_pos = 0;
            let mut current_max = 0;
            for (pos, leaf) in datasets.iter().enumerate() {
                let common = next_leaf.count_common(leaf);
                if common > current_max {
                    current_max = common;
                    similar_leaf_pos = pos;
                }
            }

            let similar_leaf = datasets.remove(similar_leaf_pos);

            let in_common = HashIntersection::from_iter(next_leaf.mins().into_iter())
                .union(&HashIntersection::from_iter(
                    similar_leaf.mins().into_iter(),
                ))
                .cloned()
                .collect();

            let simleaf_tree = BinaryTree::Dataset(Box::new(TreeNode {
                element: similar_leaf,
                left: BinaryTree::Empty,
                right: BinaryTree::Empty,
            }));
            (simleaf_tree, in_common)
        };

        let leaf_tree = BinaryTree::Dataset(Box::new(TreeNode {
            element: next_leaf,
            left: BinaryTree::Empty,
            right: BinaryTree::Empty,
        }));

        let tree = BinaryTree::Internal(Box::new(TreeNode {
            element: in_common,
            left: leaf_tree,
            right: simleaf_tree,
        }));

        next_round.push(tree);

        if next_round.len() % 100 == 0 {
            eprintln!("Processed {} leaves", next_round.len() * 2);
        }
    }
    eprintln!("Finished processing leaves");

    // while we don't get to the root, generate intermediary levels
    while next_round.len() != 1 {
        next_round = BinaryTree::process_internal_level(next_round);
        eprintln!("Finished processing round {}", next_round.len());
    }

    // Convert from binary tree to nodes/leaves
    let root = next_round.pop().unwrap();
    let mut visited = HashSet::new();
    let mut queue = vec![(0u64, root)];

    while !queue.is_empty() {
        let (pos, cnode) = queue.pop().unwrap();
        if !visited.contains(&pos) {
            visited.insert(pos);

            match cnode {
                BinaryTree::Dataset(leaf) => {
                    leaves.insert(pos, leaf.element);
                }
                BinaryTree::Internal(mut node) => {
                    let left = std::mem::replace(&mut node.left, BinaryTree::Empty);
                    let right = std::mem::replace(&mut node.right, BinaryTree::Empty);
                    queue.push((2 * pos + 1, left));
                    queue.push((2 * pos + 2, right));
                }
                BinaryTree::Empty => (),
            }
        }
    }

    // save the new tree

    let storage: Arc<dyn Storage> = Arc::new(FSStorage {
        basepath: ".sbt".into(),
    });

    SBTBuilder::default()
        .storage(storage)
        .nodes(HashMap::default())
        .leaves(leaves)
        .build()
        .unwrap()
}

impl BinaryTree {
    fn process_internal_level(mut current_round: Vec<BinaryTree>) -> Vec<BinaryTree> {
        let mut next_round = Vec::with_capacity(current_round.len() + 1);

        while !current_round.is_empty() {
            let next_node = current_round.pop().unwrap();

            let similar_node = if current_round.is_empty() {
                BinaryTree::Empty
            } else {
                let mut similar_node_pos = 0;
                let mut current_max = 0;
                for (pos, cmpe) in current_round.iter().enumerate() {
                    let common = BinaryTree::intersection_size(&next_node, &cmpe);
                    if common > current_max {
                        current_max = common;
                        similar_node_pos = pos;
                    }
                }
                current_round.remove(similar_node_pos)
            };

            let tree = BinaryTree::new_tree(next_node, similar_node);

            next_round.push(tree);
        }
        next_round
    }

    fn new_tree(mut left: BinaryTree, mut right: BinaryTree) -> BinaryTree {
        let in_common = if let BinaryTree::Internal(ref mut el1) = left {
            match right {
                BinaryTree::Internal(ref mut el2) => {
                    let c1 = std::mem::replace(&mut el1.element, HashIntersection::default());
                    let c2 = std::mem::replace(&mut el2.element, HashIntersection::default());
                    c1.union(&c2).cloned().collect()
                }
                BinaryTree::Empty => {
                    std::mem::replace(&mut el1.element, HashIntersection::default())
                }
                _ => panic!("Should not see a Dataset at this level"),
            }
        } else {
            HashIntersection::default()
        };

        BinaryTree::Internal(Box::new(TreeNode {
            element: in_common,
            left,
            right,
        }))
    }

    fn intersection_size(n1: &BinaryTree, n2: &BinaryTree) -> usize {
        if let BinaryTree::Internal(ref el1) = n1 {
            if let BinaryTree::Internal(ref el2) = n2 {
                return el1.element.intersection(&el2.element).count();
            }
        };
        0
    }
}

#[cfg(test)]
mod test {
    use std::io::{Seek, SeekFrom};
    use tempfile;

    use super::*;
    use crate::index::linear::{LinearIndex, LinearIndexBuilder};
    use crate::index::search::{search_minhashes, search_minhashes_containment};

    #[test]
    fn save_sbt() {
        let mut filename = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        filename.push("tests/test-data/v5.sbt.json");

        let sbt = MHBT::from_path(filename).expect("Loading error");

        let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
        sbt.save_file(tmpfile.path()).unwrap();

        tmpfile.seek(SeekFrom::Start(0)).unwrap();

        let mut sbt = MHBT::from_path(tmpfile.path()).expect("Loading error");
    }

    #[test]
    fn load_sbt() {
        let mut filename = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        filename.push("tests/test-data/v5.sbt.json");

        let sbt = MHBT::from_path(filename).expect("Loading error");

        assert_eq!(sbt.d, 2);
        //assert_eq!(sbt.storage.backend, "FSStorage");
        //assert_eq!(sbt.storage.args["path"], ".sbt.v5");
        assert_eq!(sbt.factory.class, "GraphFactory");
        assert_eq!(sbt.factory.args, [1, 100000, 4]);

        println!("sbt leaves {:?} {:?}", sbt.leaves.len(), sbt.leaves);

        let mut filename = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        filename.push("tests/test-data/.sbt.v3/0107d767a345eff67ecdaed2ee5cd7ba");

        let sig = Signature::from_path(filename).expect("Loading error");
        let leaf: Dataset<Signature> = sig[0].clone().into();

        let results = sbt.find(search_minhashes, &leaf, 0.5).unwrap();
        assert_eq!(results.len(), 1);
        println!("results: {:?}", results);
        println!("leaf: {:?}", leaf);

        let results = sbt.find(search_minhashes, &leaf, 0.1).unwrap();
        assert_eq!(results.len(), 3);
        println!("results: {:?}", results);
        println!("leaf: {:?}", leaf);

        let mut linear = LinearIndexBuilder::default()
            .storage(Arc::clone(&sbt.storage) as Arc<dyn Storage>)
            .build()
            .unwrap();
        for (_, l) in &sbt.leaves {
            linear.insert(l);
        }

        println!(
            "linear leaves {:?} {:?}",
            linear.datasets.len(),
            linear.datasets
        );

        let results = linear.find(search_minhashes, &leaf, 0.5).unwrap();
        assert_eq!(results.len(), 1);
        println!("results: {:?}", results);
        println!("leaf: {:?}", leaf);

        let results = linear.find(search_minhashes, &leaf, 0.1).unwrap();
        assert_eq!(results.len(), 3);
        println!("results: {:?}", results);
        println!("leaf: {:?}", leaf);

        let results = linear
            .find(search_minhashes_containment, &leaf, 0.5)
            .unwrap();
        assert_eq!(results.len(), 3);
        println!("results: {:?}", results);
        println!("leaf: {:?}", leaf);

        let results = linear
            .find(search_minhashes_containment, &leaf, 0.1)
            .unwrap();
        assert_eq!(results.len(), 3);
        println!("results: {:?}", results);
        println!("leaf: {:?}", leaf);
    }

    #[test]
    fn scaffold_sbt() {
        let mut filename = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        filename.push("tests/test-data/v5.sbt.json");

        let sbt = MHBT::from_path(filename).expect("Loading error");

        let new_sbt: MHBT = scaffold(sbt.datasets());

        assert_eq!(new_sbt.datasets().len(), 7);
    }
}

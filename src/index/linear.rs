use std::path::Path;
use std::sync::Arc;

use derive_builder::Builder;
use failure::Error;
use rayon::prelude::*;

use crate::index::storage::Storage;
use crate::index::{Comparable, Index};

#[derive(Builder)]
pub struct LinearIndex<L> {
    //#[builder(setter(skip))]
    storage: Arc<dyn Storage>,

    #[builder(setter(skip))]
    pub(crate) datasets: Vec<L>,
}

impl<L> Index for LinearIndex<L>
where
    L: Clone + Comparable<L> + Send + Sync,
{
    type Item = L;

    fn find<F>(
        &self,
        search_fn: F,
        sig: &Self::Item,
        threshold: f64,
    ) -> Result<Vec<&Self::Item>, Error>
    where
        F: Fn(&dyn Comparable<Self::Item>, &Self::Item, f64) -> bool + Send + Sync,
    {
        Ok(self
            .datasets
            .par_iter()
            .flat_map(|node| {
                if search_fn(node, sig, threshold) {
                    Some(node)
                } else {
                    None
                }
            })
            .collect())
    }

    fn insert(&mut self, node: &L) {
        self.datasets.push(node.clone());
    }

    fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), Error> {
        Ok(())
    }

    fn load<P: AsRef<Path>>(path: P) -> Result<(), Error> {
        Ok(())
    }

    fn datasets(&self) -> Vec<Self::Item> {
        self.datasets.to_vec()
    }
}

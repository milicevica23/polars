//! Used to speed up PartialEq and PartialOrd of elements within an array

use std::cmp::{Ordering, PartialEq};

use crate::chunked_array::ChunkedArrayLayout;
use crate::prelude::*;

#[repr(transparent)]
struct NonNull<T>(T);

trait GetInner {
    type Item;
    unsafe fn get_unchecked(&self, idx: usize) -> Self::Item;
}

impl<'a, T: PolarsDataType> GetInner for &'a ChunkedArray<T> {
    type Item = Option<T::Physical<'a>>;
    unsafe fn get_unchecked(&self, idx: usize) -> Self::Item {
        ChunkedArray::get_unchecked(self, idx)
    }
}

impl<'a, T: StaticArray> GetInner for &'a T {
    type Item = Option<T::ValueT<'a>>;
    unsafe fn get_unchecked(&self, idx: usize) -> Self::Item {
        <T as StaticArray>::get_unchecked(self, idx)
    }
}

impl<'a, T: PolarsDataType> GetInner for NonNull<&'a ChunkedArray<T>> {
    type Item = T::Physical<'a>;
    unsafe fn get_unchecked(&self, idx: usize) -> Self::Item {
        self.0.value_unchecked(idx)
    }
}

impl<'a, T: StaticArray> GetInner for NonNull<&'a T> {
    type Item = T::ValueT<'a>;
    unsafe fn get_unchecked(&self, idx: usize) -> Self::Item {
        self.0.value_unchecked(idx)
    }
}

pub trait PartialEqInner: Send + Sync {
    /// # Safety
    /// Does not do any bound checks.
    unsafe fn eq_element_unchecked(&self, idx_a: usize, idx_b: usize) -> bool;
}

pub trait PartialOrdInner: Send + Sync {
    /// # Safety
    /// Does not do any bound checks.
    unsafe fn cmp_element_unchecked(&self, idx_a: usize, idx_b: usize) -> Ordering;
}

impl<T> PartialEqInner for T
where
    T: GetInner + Send + Sync,
    T::Item: PartialEq,
{
    #[inline]
    unsafe fn eq_element_unchecked(&self, idx_a: usize, idx_b: usize) -> bool {
        self.get_unchecked(idx_a) == self.get_unchecked(idx_b)
    }
}

/// Create a type that implements PartialEqInner.
pub(crate) trait IntoPartialEqInner<'a> {
    /// Create a type that implements `TakeRandom`.
    fn into_partial_eq_inner(self) -> Box<dyn PartialEqInner + 'a>;
}

/// We use a trait object because we want to call this from Series and cannot use a typed enum.
impl<'a, T> IntoPartialEqInner<'a> for &'a ChunkedArray<T>
where
    T: PolarsDataType,
    T::Physical<'a>: PartialEq,
{
    fn into_partial_eq_inner(self) -> Box<dyn PartialEqInner + 'a> {
        match self.layout() {
            ChunkedArrayLayout::SingleNoNull(arr) => Box::new(NonNull(arr)),
            ChunkedArrayLayout::Single(arr) => Box::new(arr),
            ChunkedArrayLayout::MultiNoNull(ca) => Box::new(NonNull(ca)),
            ChunkedArrayLayout::Multi(ca) => Box::new(ca),
        }
    }
}

// Partial ordering implementations.
#[inline]
fn fallback<T: PartialEq>(a: T) -> Ordering {
    // This is a simple way to check if it is nan
    // without convincing the compiler we deal with floats.
    #[allow(clippy::eq_op)]
    if a != a {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

impl<T> PartialOrdInner for T
where
    T: GetInner + Send + Sync,
    T::Item: PartialOrd,
{
    #[inline]
    unsafe fn cmp_element_unchecked(&self, idx_a: usize, idx_b: usize) -> Ordering {
        let a = self.get_unchecked(idx_a);
        let b = self.get_unchecked(idx_b);
        a.partial_cmp(&b).unwrap_or_else(|| fallback(a))
    }
}

/// Create a type that implements PartialOrdInner.
pub(crate) trait IntoPartialOrdInner<'a> {
    /// Create a type that implements `TakeRandom`.
    fn into_partial_ord_inner(self) -> Box<dyn PartialOrdInner + 'a>;
}

impl<'a, T> IntoPartialOrdInner<'a> for &'a ChunkedArray<T>
where
    T: PolarsDataType,
    T::Physical<'a>: PartialOrd,
{
    fn into_partial_ord_inner(self) -> Box<dyn PartialOrdInner + 'a> {
        match self.layout() {
            ChunkedArrayLayout::SingleNoNull(arr) => Box::new(NonNull(arr)),
            ChunkedArrayLayout::Single(arr) => Box::new(arr),
            ChunkedArrayLayout::MultiNoNull(ca) => Box::new(NonNull(ca)),
            ChunkedArrayLayout::Multi(ca) => Box::new(ca),
        }
    }
}

#[cfg(feature = "dtype-categorical")]
struct LocalCategorical<'a> {
    rev_map: &'a Utf8Array<i64>,
    cats: &'a UInt32Chunked,
}

#[cfg(feature = "dtype-categorical")]
impl<'a> GetInner for LocalCategorical<'a> {
    type Item = Option<&'a str>;
    unsafe fn get_unchecked(&self, idx: usize) -> Self::Item {
        let cat = self.cats.get_unchecked(idx)?;
        Some(self.rev_map.value_unchecked(cat as usize))
    }
}

#[cfg(feature = "dtype-categorical")]
struct GlobalCategorical<'a> {
    p1: &'a PlHashMap<u32, u32>,
    p2: &'a Utf8Array<i64>,
    cats: &'a UInt32Chunked,
}

#[cfg(feature = "dtype-categorical")]
impl<'a> GetInner for GlobalCategorical<'a> {
    type Item = Option<&'a str>;
    unsafe fn get_unchecked(&self, idx: usize) -> Self::Item {
        let cat = self.cats.get_unchecked(idx)?;
        let idx = self.p1.get(&cat).unwrap();
        Some(self.p2.value_unchecked(*idx as usize))
    }
}

#[cfg(feature = "dtype-categorical")]
impl<'a> IntoPartialOrdInner<'a> for &'a CategoricalChunked {
    fn into_partial_ord_inner(self) -> Box<dyn PartialOrdInner + 'a> {
        let cats = self.logical();
        match &**self.get_rev_map() {
            RevMapping::Global(p1, p2, _) => Box::new(GlobalCategorical { p1, p2, cats }),
            RevMapping::Local(rev_map) => Box::new(LocalCategorical { rev_map, cats }),
        }
    }
}

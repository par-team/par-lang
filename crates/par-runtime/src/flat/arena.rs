use super::runtime::{Global, Package};
use crate::flat::{
    runtime::PackageBody,
    show::{Showable, Shower},
};
use arcstr::ArcStr;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::OnceLock;

/// The `Arena` is a store for values from a finite set of types,
/// and returns indices into the arena. Allocation is done using [`Arena::alloc`],
/// and values can be accessed later with [`Arena::get`]
#[derive(Serialize, Deserialize)]
pub struct Arena<Ext: Clone> {
    pub(crate) nodes: Vec<Global<Ext>>,
    pub(crate) strings: Vec<ArcStr>,
    pub(crate) string_to_location: BTreeMap<String, Index<Ext, str>>,
    pub(crate) case_branches: Vec<(Index<Ext, str>, PackageBody<Ext>)>,
    #[serde(
        serialize_with = "serialize_packages",
        deserialize_with = "deserialize_packages"
    )]
    pub(crate) packages: Vec<OnceLock<Package<Ext>>>,
    pub(crate) redexes: Vec<(Index<Ext, Global<Ext>>, Index<Ext, Global<Ext>>)>,
}

fn serialize_packages<S, Ext: Clone + Serialize>(
    value: &Vec<OnceLock<Package<Ext>>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let unwrapped: Vec<&Package<Ext>> = value.iter().map(|v| v.get().unwrap()).collect();

    unwrapped.serialize(serializer)
}

fn deserialize_packages<'de, D, Ext: Clone + Deserialize<'de>>(
    deserializer: D,
) -> Result<Vec<OnceLock<Package<Ext>>>, D::Error>
where
    D: Deserializer<'de>,
{
    let values: Vec<Package<Ext>> = Vec::deserialize(deserializer)?;

    Ok(values
        .into_iter()
        .map(|value| {
            let lock = OnceLock::new();
            lock.set(value).ok().unwrap(); // always succeeds for fresh lock
            lock
        })
        .collect())
}

impl<Ext: Clone> Default for Arena<Ext> {
    fn default() -> Self {
        Self {
            nodes: vec![],
            // Index 0 is the sentinel used for `else` case branches. Reserve it so
            // the first real signal cannot alias the sentinel.
            strings: vec![ArcStr::from("")],
            string_to_location: BTreeMap::new(),
            case_branches: vec![],
            packages: vec![],
            redexes: vec![],
        }
    }
}

impl<Ext: Clone> Arena<Ext> {
    /// Get a reference
    pub fn get<T: Indexable<Ext> + ?Sized>(&self, index: Index<Ext, T>) -> &T {
        T::get(self, index)
    }
    pub fn alloc<T: Indexable<Ext>>(&mut self, data: T) -> Index<Ext, T> {
        T::alloc(self, data)
    }
    pub fn alloc_clone<T: Indexable<Ext> + ?Sized>(&mut self, data: &T) -> Index<Ext, T> {
        T::alloc_clone(self, data)
    }
    pub fn memory_size(&self) -> usize {
        self.nodes.len() * size_of::<Global<Ext>>()
            + self.strings.len()
            + self.packages.len() * size_of::<OnceLock<Package<Ext>>>()
            + self.redexes.len() * size_of::<(Global<Ext>, Global<Ext>)>()
            + self.case_branches.len() * size_of::<(Index<Ext, str>, PackageBody<Ext>)>()
    }
    pub fn empty_string(&self) -> Index<Ext, str> {
        Index(0)
    }
    pub fn intern(&mut self, s: &ArcStr) -> Index<Ext, str> {
        if s.is_empty() {
            self.empty_string()
        } else if let Some(s) = self.string_to_location.get(s.as_str()) {
            s.clone()
        } else {
            let i = Index(self.strings.len());
            self.strings.push(s.clone());
            self.string_to_location.insert(s.to_string(), i.clone());
            i
        }
    }
    pub fn interned(&self, s: &ArcStr) -> Option<Index<Ext, str>> {
        if s.is_empty() {
            Some(self.empty_string())
        } else {
            self.string_to_location.get(s.as_str()).cloned()
        }
    }
    pub fn get_arcstr<'s>(&'s self, index: Index<Ext, str>) -> &'s ArcStr {
        &self.strings[index.0]
    }
}

impl<Ext: Clone> std::fmt::Display for Arena<Ext> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let shower = Shower::from_arena(self);
        for (idx, package) in self.packages.iter().enumerate() {
            let Some(lock) = package.get() else {
                write!(f, "@{} = <unfilled>\n", idx)?;
                continue;
            };
            let lock = &lock.body;
            if lock.debug_name.len() > 0 {
                write!(f, "// {}\n", lock.debug_name)?;
            }
            write!(f, "@{} = {}", idx, Showable(&lock.root, &shower))?;
            write!(f, "\n    $ {}", Showable(&lock.captures, &shower))?;
            for (a, b) in self.get(lock.redexes.clone()) {
                write!(
                    f,
                    "\n     {} ~ {}",
                    Showable(a, &shower),
                    Showable(b, &shower)
                )?;
            }
            write!(f, "\n")?;
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
pub struct Index<Ext: Clone, T: Indexable<Ext> + ?Sized>(pub T::Store);

impl<Ext: Clone, T: Indexable<Ext> + ?Sized> Copy for Index<Ext, T> {}

/// The `Indexable` trait is implemented by all types that are contained by an `Arena`.
/// It defines a [`Indexable::Store`] associated type, which determines what is needed to
/// index into a value of this type. For example, sized values usually require a `usize` to index them,
/// which represents the offset into the array that contains it
/// but a slice type requires a pair of offset and length.
pub trait Indexable<Ext: Clone> {
    type Store: Copy + PartialEq + Eq + PartialOrd + Ord + Serialize;
    fn get<'s>(store: &'s Arena<Ext>, index: Index<Ext, Self>) -> &'s Self;
    fn alloc<'s>(store: &'s mut Arena<Ext>, data: Self) -> Index<Ext, Self>
    where
        Self: Sized;
    fn alloc_clone<'s>(_store: &'s mut Arena<Ext>, _data: &Self) -> Index<Ext, Self> {
        todo!()
    }
}

macro_rules! slice_indexable {
    ($field:ident, $element:ty) => {
        impl<Ext: Clone> Indexable<Ext> for [$element] {
            type Store = (usize, usize);
            fn get<'s>(store: &'s Arena<Ext>, index: Index<Ext, Self>) -> &'s Self {
                &store.$field[index.0.0..index.0.0 + index.0.1]
            }
            fn alloc_clone<'s>(store: &'s mut Arena<Ext>, data: &Self) -> Index<Ext, Self> {
                let start = store.$field.len();
                store.$field.extend_from_slice(data);
                Index((start, data.len()))
            }
        }
        impl<Ext: Clone> Iterator for Index<Ext, [$element]> {
            type Item = Index<Ext, $element>;
            fn next(&mut self) -> Option<Index<Ext, $element>> {
                if self.0.1 == 0 {
                    None
                } else {
                    let ret = Index(self.0.0);
                    self.0.0 += 1;
                    self.0.1 -= 1;
                    Some(ret)
                }
            }
        }
    };
}
macro_rules! sized_indexable {
    ($field:ident, $element:ty) => {
        impl<Ext: Clone> Indexable<Ext> for $element {
            type Store = usize;
            fn get<'s>(store: &'s Arena<Ext>, index: Index<Ext, Self>) -> &'s Self {
                &store.$field[index.0]
            }
            fn alloc<'s>(store: &'s mut Arena<Ext>, data: Self) -> Index<Ext, Self> {
                let start = store.$field.len();
                store.$field.push(data);
                Index(start)
            }
        }
    };
}
slice_indexable!(case_branches, (Index<Ext, str>, PackageBody<Ext>));
slice_indexable!(nodes, Global<Ext>);
slice_indexable!(redexes, (Index<Ext, Global<Ext>>, Index<Ext, Global<Ext>>));
sized_indexable!(redexes, (Index<Ext, Global<Ext>>, Index<Ext, Global<Ext>>));
sized_indexable!(case_branches, (Index<Ext, str>, PackageBody<Ext>));
sized_indexable!(packages, OnceLock<Package<Ext>>);
sized_indexable!(nodes, Global<Ext>);

impl<Ext: Clone> Indexable<Ext> for str {
    type Store = usize;
    fn get<'s>(store: &'s Arena<Ext>, index: Index<Ext, Self>) -> &'s Self {
        &store.strings[index.0]
    }
    fn alloc_clone<'s>(store: &'s mut Arena<Ext>, data: &Self) -> Index<Ext, Self> {
        let index = store.strings.len();
        store.strings.push(data.into());
        Index(index)
    }
}

impl<Ext: Clone, T: Indexable<Ext> + ?Sized> Clone for Index<Ext, T>
where
    T::Store: Clone,
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<Ext: Clone, T: Indexable<Ext> + ?Sized> Debug for Index<Ext, T>
where
    T::Store: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<Ext: Clone, T: Indexable<Ext> + ?Sized> PartialEq for Index<Ext, T>
where
    T::Store: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
    fn ne(&self, other: &Self) -> bool {
        self.0.ne(&other.0)
    }
}
impl<Ext: Clone, T: Indexable<Ext> + ?Sized> Eq for Index<Ext, T> where T::Store: Eq {}

impl<Ext: Clone, T: Indexable<Ext> + ?Sized> PartialOrd for Index<Ext, T>
where
    T::Store: PartialOrd,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(&other.0)
    }
}
impl<Ext: Clone, T: Indexable<Ext> + ?Sized> Ord for Index<Ext, T>
where
    T::Store: Ord,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

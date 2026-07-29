use std::fmt::Display;

use crate::flat::arena::Arena;
use crate::flat::runtime::{
    Global, GlobalPtr, Linear, Node, PackageBody, Shared, SyncShared, Value,
};

pub(crate) struct Shower<'a, Ext: Clone> {
    pub arena: &'a Arena<Ext>,
    pub deref_globals: bool,
}

impl<'a, Ext: Clone> Shower<'a, Ext> {
    pub(crate) fn from_arena(arena: &'a Arena<Ext>) -> Self {
        Self {
            arena,
            deref_globals: true,
        }
    }
}

pub(crate) struct Showable<'a, 'b, P, Ext: Clone>(pub P, pub &'b Shower<'a, Ext>);
//pub struct ShowableGlobal<'a, 'b>(&'a Instance, &'a Global, &'b mut Shower<'a>);

impl<'a, 'b, Ext: Clone> std::fmt::Display for Showable<'a, 'b, &'a Node<Ext>, Ext> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Node::Empty => unreachable!(),
            Node::Linear(linear) => write!(f, "-{}", Showable(linear, self.1)),
            Node::Shared(shared) => write!(f, "&{}", Showable(shared, self.1)),
            Node::Global(_, global) => write!(f, "'{}", Showable(global, self.1)),
        }
    }
}

impl<'a, 'b, Ext: Clone> std::fmt::Display for Showable<'a, 'b, &'a Box<Node<Ext>>, Ext> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Showable(self.0.as_ref(), self.1).fmt(f)
    }
}

impl<'a, 'b, Ext: Clone> std::fmt::Display for Showable<'a, 'b, &'a Linear<Ext>, Ext> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Linear::Value(value) => {
                write!(f, "{}", Showable(value.as_ref(), self.1))?;
            }
            Linear::Continue => {
                write!(f, "?")?;
            }
            Linear::Par(a, b) => {
                write!(
                    f,
                    "[{}] {}",
                    Showable(b.as_ref(), self.1),
                    Showable(a.as_ref(), self.1)
                )?;
            }
            Linear::Request(_sender) => {
                write!(f, "<external request>")?;
            }
            Linear::Variable(_mutex) => {
                write!(f, "<external variable>")?;
            }
            Linear::ShareHole(mutex) => match mutex.try_lock() {
                Ok(lock) => match &*lock {
                    crate::flat::runtime::SharedHole::Filled(_sync_shared) => {
                        write!(f, "<unexpected filled share hole>")?;
                    }
                    crate::flat::runtime::SharedHole::Unfilled(_nodes) => {
                        write!(f, "<unfilled hole>")?;
                    }
                },
                _ => {
                    write!(f, "<locked>")?;
                }
            },
        }
        Ok(())
    }
}

impl<'a, 'b, Ext: Clone> std::fmt::Display for Showable<'a, 'b, &'a Shared<Ext>, Ext> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Shared::Async(mutex) => match mutex.try_lock() {
                Ok(lock) => match &*lock {
                    crate::flat::runtime::SharedHole::Filled(sync_shared) => {
                        write!(f, "{}", Showable(sync_shared, self.1))?;
                    }
                    crate::flat::runtime::SharedHole::Unfilled(_) => {
                        write!(f, "<waiting>")?;
                    }
                },
                _ => {
                    write!(f, "<locked>")?;
                }
            },
            Shared::Sync(sync_shared) => {
                write!(f, "{}", Showable(sync_shared.as_ref(), self.1))?;
            }
        };
        Ok(())
    }
}
impl<'a, 'b, Ext: Clone> std::fmt::Display for Showable<'a, 'b, &'a SyncShared<Ext>, Ext> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            SyncShared::Package(index, shared) => {
                write!(f, "@{}${}", index.0, Showable(shared, self.1))?;
            }
            SyncShared::Value(value) => {
                write!(f, "{}", Showable(value, self.1))?;
            }
        };
        Ok(())
    }
}

impl<'a, 'b, Ext: Clone> std::fmt::Display for Showable<'a, 'b, &'a Global<Ext>, Ext> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Global::Variable(id) => {
                write!(f, "{}", id)?;
            }
            Global::Close { .. } => {
                write!(f, "close")?;
            }
            Global::Package(index, captures, _) => {
                write!(f, "@{}${}", index.0, Showable(captures, self.1))?;
            }
            Global::Fanout(index) => {
                write!(f, "{{")?;
                for i in self.1.arena.get(index.clone()) {
                    write!(f, "{} ", Showable(i, self.1))?;
                }
                write!(f, "}}")?;
            }
            Global::Destruct(global_cont) => {
                use crate::flat::runtime::GlobalCont::*;
                match global_cont {
                    Continue => write!(f, "?")?,
                    Par(a, b) => {
                        write!(f, "[{}] {}", Showable(b, self.1), Showable(a, self.1))?;
                    }
                    Choice(captures, branches) => {
                        write!(f, ".{{")?;
                        for (k, v) in self.1.arena.get(branches.clone()).iter() {
                            write!(
                                f,
                                "{} @{} ",
                                self.1.arena.get(k.clone()),
                                Showable(v, self.1)
                            )?;
                        }
                        write!(f, "}}${}", Showable(captures, self.1))?;
                    }
                }
            }
            Global::Value(value) => {
                write!(f, "{}", Showable(value, self.1))?;
            }
        };
        Ok(())
    }
}

impl<'a, 'b, Ext: Clone> std::fmt::Display for Showable<'a, 'b, &'a GlobalPtr<Ext>, Ext> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.1.deref_globals {
            write!(f, "{}", Showable(self.1.arena.get(self.0.clone()), self.1))?;
        } else {
            write!(f, "{}", self.0.0)?;
        }
        Ok(())
    }
}

impl<'a, 'b, P, Ext: Clone> std::fmt::Display for Showable<'a, 'b, &'a Value<P, Ext>, Ext>
where
    Showable<'a, 'b, &'a P, Ext>: Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::flat::runtime::Value::*;
        match self.0 {
            Break => write!(f, "!")?,
            Pair(a, b) => {
                write!(f, "({}) {}", Showable(b, self.1), Showable(a, self.1))?;
            }
            Either(arc_str, payload) => {
                write!(
                    f,
                    ".{} {}",
                    self.1.arena.get(arc_str.clone()),
                    Showable(payload, self.1)
                )?;
            }
            ExternalFn(_) => {
                write!(f, "<external fn>")?;
            }
            ExternalArc(_) => {
                write!(f, "<external arc>")?;
            }
            Primitive(primitive) => {
                write!(f, "#{:?}", primitive)?;
            }
        };
        Ok(())
    }
}

use super::runtime::Package;
use std::sync::OnceLock;

impl<'a, 'b, Ext: Clone> std::fmt::Display for Showable<'a, 'b, &'a OnceLock<Package<Ext>>, Ext> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let package = self.0;

        let Some(lock) = package.get() else {
            write!(f, "<unfilled>")?;
            return Ok(());
        };
        write!(f, "{}", Showable(&lock.body, self.1))?;
        Ok(())
    }
}

impl<'a, 'b, Ext: Clone> std::fmt::Display for Showable<'a, 'b, &'a PackageBody<Ext>, Ext> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let package = self.0;
        if package.debug_name.len() > 0 {
            write!(f, "/* {} */", package.debug_name)?;
        }
        write!(f, "@{}", Showable(&package.root, self.1))?;
        write!(f, "${}", Showable(&package.captures, self.1))?;
        for (a, b) in self.1.arena.get(package.redexes.clone()) {
            write!(f, "& {} ~ {}", Showable(a, self.1), Showable(b, &self.1))?;
        }
        Ok(())
    }
}

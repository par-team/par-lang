use super::super::language::GlobalName;
use super::core::Type;
use crate::frontend_impl::types::visit;

impl<S: Clone> Type<S> {
    pub fn get_dependencies(&self) -> Vec<GlobalName<S>> {
        fn inner<S: Clone>(typ: &Type<S>, deps: &mut Vec<GlobalName<S>>) -> Result<(), ()> {
            match typ {
                Type::Name(_, name, args)
                | Type::DualName(_, name, args)
                | Type::SizedName(_, _, name, args)
                | Type::SizedDualName(_, _, name, args) => {
                    deps.push(name.clone());
                    for arg in args {
                        inner(arg, deps)?;
                    }
                }
                _ => {
                    visit::continue_(typ, |child| inner(child, deps))?;
                }
            }
            Ok(())
        }
        let mut res = vec![];
        inner(self, &mut res).unwrap();
        res
    }
}

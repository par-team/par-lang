pub mod source {
    pub use crate::location::{FileName, Point, Span, Spanning};
}

pub mod frontend {
    use crate::frontend_impl::language::{CompileError, Context};
    use crate::frontend_impl::parse::{parse_module, parse_source_file as parse_source_file_impl};
    use crate::location::FileName;
    use crate::runtime_impl::{Compiled, RuntimeCompilerError};
    use par_runtime::linker::Unlinked;
    use std::sync::Arc;

    pub mod language {
        pub use crate::frontend_impl::language::*;
    }

    pub mod process {
        pub use crate::frontend_impl::process::*;
    }

    pub use crate::frontend_impl::parse::SyntaxError;
    pub use crate::frontend_impl::parse_bytes;
    pub use crate::frontend_impl::program::{
        CheckedModule, Declaration, Definition, DefinitionBody, DocComment, Docs, HoverIndex,
        ImportDecl, ImportPath, Module, ModuleDecl, ParseAndCompileError, SourceFile, TypeDef,
    };
    pub use crate::frontend_impl::set_miette_hook;
    pub use crate::frontend_impl::types::lattice::{intersect_types, union_types};
    pub use crate::frontend_impl::types::registry::{ExternalTypeDef, get_external_type_defs};
    pub use crate::frontend_impl::types::visibility::Visibility;
    pub use crate::frontend_impl::types::{
        GlobalNameWriter, Operation, PrimitiveType, Type, TypeBranch, TypeDefs, TypeError,
    };
    pub use par_runtime::data::Data;
    pub use par_runtime::primitive::{Number, ParString, Primitive};

    pub type HighLevelModule =
        Module<language::Expression<language::Unresolved>, language::Unresolved>;
    pub type LowLevelModule =
        Module<Arc<process::Expression<(), language::Universal>>, language::Universal>;
    pub type LowLevelUnresolvedModule =
        Module<Arc<process::Expression<(), language::Unresolved>>, language::Unresolved>;

    pub fn parse(source: &str, file: FileName) -> Result<HighLevelModule, SyntaxError> {
        parse_module(source, file)
    }

    pub fn parse_source_file(
        source: &str,
        file: FileName,
    ) -> Result<SourceFile<language::Expression<language::Unresolved>>, SyntaxError> {
        parse_source_file_impl(source, file)
    }

    pub fn is_lowercase_identifier(identifier: &str) -> bool {
        crate::frontend_impl::lexer::is_lowercase_identifier(identifier)
    }

    pub fn lower(module: HighLevelModule) -> Result<LowLevelUnresolvedModule, CompileError> {
        let compiled_definitions = module
            .definitions
            .into_iter()
            .map(|Definition { span, name, body }| {
                Ok(Definition {
                    span,
                    name,
                    body: match body {
                        DefinitionBody::Par(expr) => {
                            let compiled = Context::new().compile_expression(&expr)?;
                            let compiled = compiled.optimize().fix_captures().0;
                            DefinitionBody::Par(compiled)
                        }
                        DefinitionBody::External(span) => DefinitionBody::External(span),
                    },
                })
            })
            .collect::<Result<_, _>>()?;

        Ok(Module {
            type_defs: module.type_defs,
            declarations: module.declarations,
            definitions: compiled_definitions,
        })
    }

    pub fn type_check(
        module: &LowLevelModule,
    ) -> (
        CheckedModule<language::Universal>,
        Vec<TypeError<language::Universal>>,
    ) {
        module.type_check()
    }

    pub fn compile_runtime(
        module: &CheckedModule<language::Universal>,
        max_interactions: u32,
    ) -> Result<Compiled<Unlinked>, RuntimeCompilerError> {
        Compiled::compile_file(module, max_interactions)
    }
}

pub mod runtime {
    pub use crate::runtime_impl::{Compiled, RuntimeCompilerError};
    pub use crate::typed_readback::{TypedHandle, TypedReadback, type_supports_readback};
    pub use par_runtime::data::Data;
    pub use par_runtime::primitive::Number;
}

pub mod execution {
    pub use par_runtime::spawn::TokioSpawn;
}

pub mod testing {
    pub use crate::test_assertion::{AssertionResult, provide_test};
}

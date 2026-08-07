#[cfg(test)]
mod tests {
    use crate::frontend_impl::language::{
        GlobalName, LocalName, TypeConstraint, TypeParameter, Universal,
    };
    use crate::frontend_impl::types::{GlobalNameWriter, Type, TypeDefs};
    use crate::location::Span;
    use crate::workspace::render_type_in_scope;
    use arcstr::{ArcStr, literal};
    use par_runtime::pkgid::PackageId;
    use std::fmt::{self, Write};

    struct TestNameWriter;

    impl GlobalNameWriter<Universal> for TestNameWriter {
        fn write_global_name<W: Write>(
            &self,
            f: &mut W,
            name: &GlobalName<Universal>,
        ) -> fmt::Result {
            write!(f, "{name}")
        }
    }

    fn alias_preserving_type_defs() -> (TypeDefs<Universal>, GlobalName<Universal>) {
        let span = Span::None;
        let key = LocalName {
            span: Span::None,
            string: ArcStr::from("k"),
        };
        let value = LocalName {
            span: Span::None,
            string: ArcStr::from("v"),
        };
        let map_name = GlobalName::new(
            Span::None,
            Universal {
                package: PackageId::Special(literal!("__test__")),
                directories: vec![],
                module: "Main".to_string(),
            },
            "Map".to_string(),
        );
        let body = Type::iterative(
            None,
            Type::choice(vec![
                ("delete", Type::self_(None)),
                (
                    "put",
                    Type::Function(
                        Span::None,
                        Box::new(Type::Var(Span::None, value.clone())),
                        Box::new(Type::self_(None)),
                        vec![],
                    ),
                ),
            ]),
        );
        let params = vec![TypeParameter::any(key), TypeParameter::any(value)];
        let (defs, errors) =
            TypeDefs::new_with_validation([(&span, &map_name, &params, &body)].into_iter());
        assert!(errors.is_empty(), "errors: {errors:?}");
        (defs, map_name)
    }

    #[test]
    fn test_iterative_box_choice() {
        let typ: Type<Universal> = Type::iterative_box_choice(
            None,
            vec![
                ("method1", Type::<Universal>::string()),
                ("method2", Type::<Universal>::int()),
            ],
        );

        match typ {
            Type::Iterative { body, .. } => match body.as_ref() {
                Type::Box(_, inner) => match inner.as_ref() {
                    Type::Choice(_, branches) => {
                        assert_eq!(branches.len(), 2);
                        assert!(branches.contains_key(
                            &crate::frontend_impl::language::LocalName {
                                span: crate::location::Span::None,
                                string: arcstr::ArcStr::from("method1"),
                            }
                        ));
                        assert!(branches.contains_key(
                            &crate::frontend_impl::language::LocalName {
                                span: crate::location::Span::None,
                                string: arcstr::ArcStr::from("method2"),
                            }
                        ));
                    }
                    _ => panic!("Expected Choice type inside Box"),
                },
                _ => panic!("Expected Box type"),
            },
            _ => panic!("Expected Iterative type"),
        }
    }

    #[test]
    fn test_iterative_box_choice_with_label() {
        let typ: Type<Universal> = Type::iterative_box_choice(
            Some("my_label"),
            vec![(
                "action",
                Type::function(Type::<Universal>::nat(), Type::<Universal>::break_()),
            )],
        );

        match typ {
            Type::Iterative { label, body, .. } => {
                assert!(label.is_some());
                assert_eq!(label.unwrap().string.as_str(), "my_label");

                match body.as_ref() {
                    Type::Box(_, inner) => match inner.as_ref() {
                        Type::Choice(_, branches) => {
                            assert_eq!(branches.len(), 1);
                        }
                        _ => panic!("Expected Choice type inside Box"),
                    },
                    _ => panic!("Expected Box type"),
                }
            }
            _ => panic!("Expected Iterative type"),
        }
    }

    #[test]
    fn test_iterative_box_choice_equivalent_to_manual() {
        let manual: Type<Universal> = Type::iterative(
            None,
            Type::box_(Type::choice(vec![("test", Type::<Universal>::string())])),
        );

        let helper: Type<Universal> =
            Type::iterative_box_choice(None, vec![("test", Type::<Universal>::string())]);

        match (manual, helper) {
            (Type::Iterative { body: body1, .. }, Type::Iterative { body: body2, .. }) => {
                match (body1.as_ref(), body2.as_ref()) {
                    (Type::Box(_, inner1), Type::Box(_, inner2)) => {
                        match (inner1.as_ref(), inner2.as_ref()) {
                            (Type::Choice(_, branches1), Type::Choice(_, branches2)) => {
                                assert_eq!(branches1.len(), branches2.len());
                            }
                            _ => panic!("Expected Choice types"),
                        }
                    }
                    _ => panic!("Expected Box types"),
                }
            }
            _ => panic!("Expected Iterative types"),
        }
    }

    #[test]
    fn test_empty_either_subtype_of_any() {
        let type_defs: TypeDefs<Universal> = TypeDefs::default();
        let empty_either: Type<Universal> = Type::either(vec![]);
        let any_type: Type<Universal> = Type::string();

        assert!(
            empty_either
                .is_definitely_assignable_to(&any_type, &type_defs)
                .unwrap()
                .is_assignable()
        );
    }

    #[test]
    fn test_any_subtype_of_empty_choice() {
        let type_defs: TypeDefs<Universal> = TypeDefs::default();
        let any_type: Type<Universal> = Type::int();
        let empty_choice: Type<Universal> = Type::choice(vec![]);

        assert!(
            any_type
                .is_definitely_assignable_to(&empty_choice, &type_defs)
                .unwrap()
                .is_assignable()
        );
    }

    fn constrained_param(constraint: TypeConstraint) -> TypeParameter {
        TypeParameter {
            name: LocalName {
                span: Span::None,
                string: ArcStr::from("a"),
            },
            constraint,
        }
    }

    #[test]
    fn test_exists_constraint_is_covariant() {
        let type_defs: TypeDefs<Universal> = TypeDefs::default();
        let exists = |constraint| {
            Type::<Universal>::Exists(
                Span::None,
                constrained_param(constraint),
                Box::new(Type::pair(Type::var("a"), Type::break_())),
            )
        };

        // A witness promising `box` may be used where no promise is needed...
        assert!(
            exists(TypeConstraint::Box)
                .is_definitely_assignable_to(&exists(TypeConstraint::Any), &type_defs)
                .unwrap()
                .is_assignable()
        );
        // ...but an unconstrained (possibly linear) witness must not be
        // passed off as a `box` one.
        assert!(
            !exists(TypeConstraint::Any)
                .is_definitely_assignable_to(&exists(TypeConstraint::Box), &type_defs)
                .unwrap()
                .is_assignable()
        );
    }

    #[test]
    fn test_forall_constraint_is_contravariant() {
        let type_defs: TypeDefs<Universal> = TypeDefs::default();
        let forall = |constraint| {
            Type::<Universal>::Forall(
                Span::None,
                constrained_param(constraint),
                Box::new(Type::function(Type::var("a"), Type::break_())),
            )
        };

        // Accepting any type is stronger than only accepting `box` types...
        assert!(
            forall(TypeConstraint::Any)
                .is_definitely_assignable_to(&forall(TypeConstraint::Box), &type_defs)
                .unwrap()
                .is_assignable()
        );
        // ...but not the other way around.
        assert!(
            !forall(TypeConstraint::Box)
                .is_definitely_assignable_to(&forall(TypeConstraint::Any), &type_defs)
                .unwrap()
                .is_assignable()
        );
    }

    #[test]
    fn test_pair_vars_constraint_is_covariant() {
        let type_defs: TypeDefs<Universal> = TypeDefs::default();
        let pair = |constraint| {
            Type::<Universal>::Pair(
                Span::None,
                Box::new(Type::var("a")),
                Box::new(Type::break_()),
                vec![constrained_param(constraint)],
            )
        };

        assert!(
            pair(TypeConstraint::Box)
                .is_definitely_assignable_to(&pair(TypeConstraint::Any), &type_defs)
                .unwrap()
                .is_assignable()
        );
        assert!(
            !pair(TypeConstraint::Any)
                .is_definitely_assignable_to(&pair(TypeConstraint::Box), &type_defs)
                .unwrap()
                .is_assignable()
        );
    }

    #[test]
    fn test_empty_branches_render_on_one_line() {
        let mut pretty_either = String::new();
        Type::<Universal>::either(vec![])
            .pretty(&mut pretty_either, &TestNameWriter, 0)
            .unwrap();
        assert_eq!(pretty_either, "either {}");

        let mut pretty_choice = String::new();
        Type::<Universal>::choice(vec![])
            .pretty(&mut pretty_choice, &TestNameWriter, 0)
            .unwrap();
        assert_eq!(pretty_choice, "choice {}");
    }

    #[test]
    fn test_implicit_generic_items_render_inside_pair_like_delimiters() {
        let parameter = |name: &str, constraint| TypeParameter {
            name: LocalName {
                span: Span::None,
                string: ArcStr::from(name),
            },
            constraint,
        };

        let function = Type::<Universal>::Function(
            Span::None,
            Box::new(Type::var("a")),
            Box::new(Type::Function(
                Span::None,
                Box::new(Type::string()),
                Box::new(Type::Function(
                    Span::None,
                    Box::new(Type::var("b")),
                    Box::new(Type::int()),
                    vec![parameter("b", TypeConstraint::Box)],
                )),
                vec![],
            )),
            vec![parameter("a", TypeConstraint::Any)],
        );
        let mut rendered = String::new();
        function
            .pretty_compact(&mut rendered, &TestNameWriter)
            .unwrap();
        assert_eq!(rendered, "[<a> a, String, <b: box> b] Int");

        let pair = Type::<Universal>::Pair(
            Span::None,
            Box::new(Type::nat()),
            Box::new(Type::Pair(
                Span::None,
                Box::new(Type::var("a")),
                Box::new(Type::Pair(
                    Span::None,
                    Box::new(Type::string()),
                    Box::new(Type::break_()),
                    vec![],
                )),
                vec![parameter("a", TypeConstraint::Data)],
            )),
            vec![],
        );
        rendered.clear();
        pair.pretty_compact(&mut rendered, &TestNameWriter).unwrap();
        assert_eq!(rendered, "(Nat, <a: data> a, String)!");
    }

    #[test]
    fn test_implicit_generic_branch_items_render_compactly() {
        let parameter = TypeParameter::any(LocalName {
            span: Span::None,
            string: ArcStr::from("a"),
        });
        let pair = Type::<Universal>::Pair(
            Span::None,
            Box::new(Type::var("a")),
            Box::new(Type::break_()),
            vec![parameter.clone()],
        );
        let function = Type::<Universal>::Function(
            Span::None,
            Box::new(Type::var("a")),
            Box::new(Type::break_()),
            vec![parameter],
        );

        let mut rendered = String::new();
        Type::either(vec![("dat", pair)])
            .pretty_compact(&mut rendered, &TestNameWriter)
            .unwrap();
        assert_eq!(rendered, "either {.dat(<a> a)!,}");

        rendered.clear();
        Type::choice(vec![("use", function)])
            .pretty_compact(&mut rendered, &TestNameWriter)
            .unwrap();
        assert_eq!(rendered, "choice {.use(<a> a) => !,}");
    }

    #[test]
    fn test_pretty_compact_keeps_named_fixpoint_aliases_after_expansion() {
        let (defs, map_name) = alias_preserving_type_defs();
        let expanded = defs
            .get(&Span::None, &map_name, &[Type::string(), Type::int()])
            .unwrap()
            .expand_fixpoint()
            .unwrap();
        let mut actual = String::new();
        expanded
            .pretty_compact(&mut actual, &TestNameWriter)
            .unwrap();

        assert_eq!(
            actual,
            "choice {.delete => @__test__/Main.Map<String, Int>,.put(Int) => @__test__/Main.Map<String, Int>,}"
        );
    }

    #[test]
    fn test_workspace_renderer_keeps_named_fixpoint_aliases_after_expansion() {
        let (defs, map_name) = alias_preserving_type_defs();
        let expanded = defs
            .get(&Span::None, &map_name, &[Type::string(), Type::int()])
            .unwrap()
            .expand_fixpoint()
            .unwrap();

        assert_eq!(
            render_type_in_scope(None, &expanded, 0),
            "\
choice {
  .delete => @__test__/Main.Map<String, Int>,
  .put(Int) => @__test__/Main.Map<String, Int>,
}"
        );
    }
}

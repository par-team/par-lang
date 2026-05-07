#[cfg(test)]
mod tests {
    use crate::frontend_impl::language::{GlobalName, LocalName, TypeParameter, Universal};
    use crate::frontend_impl::types::{GlobalNameWriter, Type, TypeDefs};
    use crate::location::Span;
    use crate::workspace::render_type_in_scope;
    use arcstr::literal;
    use par_runtime::atom::Atom;
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
            string: Atom::from("k"),
        };
        let value = LocalName {
            span: Span::None,
            string: Atom::from("v"),
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
                                string: Atom::from("method1"),
                            }
                        ));
                        assert!(branches.contains_key(
                            &crate::frontend_impl::language::LocalName {
                                span: crate::location::Span::None,
                                string: Atom::from("method2"),
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
                assert_eq!(&label.unwrap().string, "my_label");

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
            "choice {.delete => @__test__/Main.Map<String, Int>,.put => [Int] @__test__/Main.Map<String, Int>,}"
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
  .put => [Int] @__test__/Main.Map<String, Int>,
}"
        );
    }
}

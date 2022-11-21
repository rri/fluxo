//! Abstract syntax tree and related data logic.

mod ctx;
mod exp;
mod var;

pub use ctx::Ctx;
pub use exp::Exp;
pub use var::{Idx, Var, VarIdx};

#[cfg(test)]
mod test {

    use super::*;
    use std::error::Error;

    #[test]
    fn test_exp_rend_0001() {
        assert_eq!(Exp::get_kind_meta().to_string(), "□");
    }

    #[test]
    fn test_exp_rend_0002() {
        assert_eq!(Exp::get_type_meta().to_string(), "*");
    }

    #[test]
    fn test_exp_rend_0003() {
        assert_eq!(Exp::new_var(Var::new("x")).to_string(), "x");
    }

    #[test]
    fn test_exp_rend_0004() {
        assert_eq!(
            Exp::new_app(Exp::new_var(Var::new("x")), Exp::new_var(Var::new("y")),).to_string(),
            "x y"
        );
    }

    #[test]
    fn test_exp_rend_0005() {
        assert_eq!(
            Exp::new_abs(
                Var::new("x"),
                Exp::new_var(Var::new("t")),
                Exp::new_var(Var::new("x")),
            )
            .to_string(),
            "λx : t . x"
        );
    }

    #[test]
    fn test_exp_rend_0006() {
        assert_eq!(
            Exp::new_for(
                Var::new("x"),
                Exp::new_var(Var::new("t")),
                Exp::new_var(Var::new("x")),
            )
            .to_string(),
            "Πx : t . x"
        );
    }

    #[test]
    fn test_exp_rend_0007() {
        assert_eq!(
            Exp::new_abs(
                Var::new("x"),
                Exp::new_var(Var::new("t")),
                Exp::new_for(
                    Var::new("y"),
                    Exp::new_var(Var::new("b")),
                    Exp::new_var(Var::new("y")),
                ),
            )
            .to_string(),
            "λx : t . Πy : b . y"
        );
    }

    #[test]
    fn test_exp_rend_0008() {
        assert_eq!(
            Exp::new_for(
                Var::new("x"),
                Exp::new_var(Var::new("t")),
                Exp::new_abs(
                    Var::new("y"),
                    Exp::new_var(Var::new("b")),
                    Exp::new_var(Var::new("y")),
                ),
            )
            .to_string(),
            "Πx : t . λy : b . y"
        );
    }

    #[test]
    fn test_exp_rend_0009() {
        assert_eq!(
            Exp::new_abs(
                Var::new("x"),
                Exp::new_var(Var::new("t")),
                Exp::new_abs(
                    Var::new("y"),
                    Exp::new_var(Var::new("b")),
                    Exp::new_var(Var::new("y")),
                ),
            )
            .to_string(),
            "λx : t . λy : b . y"
        );
    }

    #[test]
    fn test_exp_rend_0010() {
        assert_eq!(
            Exp::new_for(
                Var::new("x"),
                Exp::get_kind_meta(),
                Exp::new_abs(
                    Var::new("y"),
                    Exp::new_var(Var::new("b")),
                    Exp::new_var(Var::new("y")),
                ),
            )
            .to_string(),
            "Πx : □ . λy : b . y"
        );
    }

    #[test]
    fn test_exp_rend_0011() {
        assert_eq!(
            Exp::new_abs(
                Var::new("x"),
                Exp::new_var(Var::new("t")),
                Exp::new_abs(
                    Var::new("y"),
                    Exp::new_var(Var::new("b")),
                    Exp::new_var(Var::new("y")),
                ),
            )
            .to_string(),
            "λx : t . λy : b . y"
        );
    }

    #[test]
    fn test_exp_rend_0012() {
        assert_eq!(
            Exp::new_app(
                Exp::new_var(Var::new("t")),
                Exp::new_abs(
                    Var::new("y"),
                    Exp::new_var(Var::new("b")),
                    Exp::new_var(Var::new("y")),
                ),
            )
            .to_string(),
            "t λy : b . y"
        );
    }

    #[test]
    fn test_exp_rend_0013() {
        assert_eq!(
            Exp::new_app(
                Exp::new_abs(
                    Var::new("y"),
                    Exp::new_var(Var::new("b")),
                    Exp::new_var(Var::new("y")),
                ),
                Exp::new_var(Var::new("t")),
            )
            .to_string(),
            "(λy : b . y) t"
        );
    }

    #[test]
    fn test_exp_rend_0014() {
        assert_eq!(
            Exp::new_app(
                Exp::new_abs(
                    Var::new("y"),
                    Exp::new_app(
                        Exp::new_abs(
                            Var::new("y"),
                            Exp::new_var(Var::new("b")),
                            Exp::new_var(Var::new("y")),
                        ),
                        Exp::new_var(Var::new("t")),
                    ),
                    Exp::new_app(Exp::new_var(Var::new("y")), Exp::new_var(Var::new("r")),),
                ),
                Exp::new_var(Var::new("t")),
            )
            .to_string(),
            "(λy : (λy : b . y) t . y r) t"
        );
    }

    #[test]
    fn test_exp_rend_0015() {
        assert_eq!(
            Exp::new_abs(
                Var::new("y"),
                Exp::new_var(Var::new("t")),
                Exp::new_app(Exp::new_var(Var::new("y")), Exp::new_var(Var::new("r")),),
            )
            .to_string(),
            "λy : t . y r"
        );
    }

    #[test]
    fn test_exp_rend_0016() {
        assert_eq!(
            Exp::new_app(
                Exp::new_var(Var::new("x")),
                Exp::new_app(Exp::new_var(Var::new("y")), Exp::new_var(Var::new("z")),),
            )
            .to_string(),
            "x (y z)"
        );
    }

    #[test]
    fn test_exp_rend_0017() {
        assert_eq!(
            Exp::new_app(
                Exp::new_app(Exp::new_var(Var::new("x")), Exp::new_var(Var::new("y")),),
                Exp::new_var(Var::new("z")),
            )
            .to_string(),
            "x y z"
        );
    }

    #[test]
    fn test_exp_rend_0018() {
        assert_eq!(
            Exp::new_abs(
                Var::new("x"),
                Exp::get_type_meta(),
                Exp::new_app(
                    Exp::new_var(Var::new("x")),
                    Exp::new_app(Exp::new_var(Var::new("y")), Exp::new_var(Var::new("z")),),
                ),
            )
            .to_string(),
            "λx : * . x (y z)"
        );
    }

    #[test]
    fn test_exp_rend_0019() {
        assert_eq!(
            Exp::new_abs(
                Var::new("x"),
                Exp::get_type_meta(),
                Exp::new_app(
                    Exp::new_app(Exp::new_var(Var::new("x")), Exp::new_var(Var::new("y")),),
                    Exp::new_var(Var::new("z")),
                ),
            )
            .to_string(),
            "λx : * . x y z"
        );
    }

    #[test]
    fn test_exp_rend_0020() {
        assert_eq!(
            Exp::new_app(
                Exp::new_var(Var::new("x")),
                Exp::new_abs(
                    Var::new("y"),
                    Exp::get_type_meta(),
                    Exp::new_abs(
                        Var::new("w"),
                        Exp::get_type_meta(),
                        Exp::new_app(Exp::new_var(Var::new("w")), Exp::new_var(Var::new("m")),),
                    ),
                ),
            )
            .to_string(),
            "x λy : * . λw : * . w m"
        );
    }

    #[test]
    fn test_exp_rend_0021() {
        assert_eq!(
            Exp::new_app(
                Exp::new_app(
                    Exp::new_var(Var::new("x")),
                    Exp::new_abs(
                        Var::new("y"),
                        Exp::get_type_meta(),
                        Exp::new_abs(
                            Var::new("w"),
                            Exp::get_type_meta(),
                            Exp::new_var(Var::new("w")),
                        ),
                    ),
                ),
                Exp::new_var(Var::new("m")),
            )
            .to_string(),
            "x (λy : * . λw : * . w) m"
        );
    }

    #[test]
    fn test_exp_rend_0022() {
        assert_eq!(
            Exp::new_app(
                Exp::new_var(Var::new("x")),
                Exp::new_abs(
                    Var::new("y"),
                    Exp::get_type_meta(),
                    Exp::new_app(
                        Exp::new_abs(
                            Var::new("w"),
                            Exp::get_type_meta(),
                            Exp::new_var(Var::new("w")),
                        ),
                        Exp::new_var(Var::new("m")),
                    ),
                ),
            )
            .to_string(),
            "x λy : * . (λw : * . w) m"
        );
    }

    #[test]
    fn test_exp_rend_0023() {
        assert_eq!(
            Exp::new_app(
                Exp::new_for(
                    Var::new("x"),
                    Exp::get_kind_meta(),
                    Exp::new_abs(
                        Var::new("m"),
                        Exp::get_type_meta(),
                        Exp::new_var(Var::new("w")),
                    ),
                ),
                Exp::new_var(Var::new("k")),
            )
            .to_string(),
            "(Πx : □ . λm : * . w) k"
        );
    }

    #[test]
    fn test_exp_idx_0001() {
        // x Πy : * . (λw : * . w y) m

        let exp = Exp::new_app(
            Exp::new_var(Var::new("x")),
            Exp::new_for(
                Var::new("y"),
                Exp::get_type_meta(),
                Exp::new_app(
                    Exp::new_abs(
                        Var::new("w"),
                        Exp::get_type_meta(),
                        Exp::new_app(Exp::new_var(Var::new("w")), Exp::new_var(Var::new("y"))),
                    ),
                    Exp::new_var(Var::new("m")),
                ),
            ),
        );

        println!("{}", exp.to_string());

        if let Exp::App(_, t0) = exp {
            if let Exp::For(_, _, t1) = *t0 {
                if let Exp::App(t2, _) = *t1 {
                    if let Exp::Abs(_, _, t3) = *t2 {
                        if let Exp::App(t4, t5) = *t3 {
                            if let Exp::Var(VarIdx::Idx(Idx(i, Var(s)))) = *t4 {
                                if i != 0 {
                                    panic!("Index should have been 0, but was {}!", i);
                                }
                                if s != "w".to_string() {
                                    panic!("Variable should have been 'w' but was {}!", s);
                                }
                            } else {
                                panic!("Expected Exp::Var!");
                            }
                            if let Exp::Var(VarIdx::Idx(Idx(i, Var(s)))) = *t5 {
                                if i != 1 {
                                    panic!("Index should have been 1, but was {}!", i);
                                }
                                if s != "y".to_string() {
                                    panic!("Variable should have been 'y' but was {}!", s);
                                }
                            } else {
                                panic!("Expected Exp::Var!");
                            }
                            return;
                        }
                        panic!("Expected Exp::App!");
                    }
                    panic!("Expected Exp::Abs!");
                }
                panic!("Expected Exp::App!");
            }
            panic!("Expected Exp::For!");
        }
        panic!("Expected Exp::App!");
    }

    #[test]
    fn test_type_calculation_001() -> Result<(), Box<dyn Error>> {
        let mut ctx = Ctx::new();
        ctx.put(&Var::new("w"), &Exp::TypeMeta)?;
        // λx : * . λm : * . w
        let can = Exp::new_abs(
            Var::new("x"),
            Exp::get_type_meta(),
            Exp::new_abs(
                Var::new("m"),
                Exp::get_type_meta(),
                Exp::new_app(
                    Exp::new_abs(
                        Var::new("k"),
                        Exp::get_type_meta(),
                        Exp::new_var(Var::new("k")),
                    ),
                    Exp::new_var(Var::new("m")),
                ),
            ),
        );
        let typ = Exp::new_for(
            Var::new("x"),
            Exp::get_type_meta(),
            Exp::new_for(Var::new("m"), Exp::get_type_meta(), Exp::get_type_meta()),
        );
        let act = can.calculate_type(&ctx)?;
        assert_eq!(act, typ);
        Ok(())
    }
}

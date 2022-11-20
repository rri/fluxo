//! Top-level expression in the core fluxo language and related logic.

use super::{Ctx, Idx, Var, VarIdx};
use crate::err::{TypeCompatErr, TypeUndefErr, TypingErr};
use std::cmp::Ordering;
use std::fmt::{Display, Formatter};

/// Top-level expression in the core fluxo language.
#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub enum Exp {
    /// Variable expression, internally holding either a symbol or a bound (and indexed) variable.
    Var(VarIdx),
    /// λ abstraction, which denotes an anonymous function.
    Abs(Var, Box<Exp>, Box<Exp>),
    /// Π type, which denotes the type of a term, type or type constructor.
    For(Var, Box<Exp>, Box<Exp>),
    /// Application of an abstraction to an expression.
    App(Box<Exp>, Box<Exp>),
    /// The type of all types, denoted by `*`.
    TypeMeta,
    /// The type of all kinds, denoted by `□`.
    KindMeta,
}

#[derive(Copy, Clone, Debug, Default)]
struct Branch {
    ltree: bool,
    rtree: bool,
}

impl Exp {
    /// Create a new instance of an [expression][Exp] that denotes a [variable][Exp::Var].
    pub fn new_var(var: Var) -> Self {
        Self::Var(VarIdx::new_var(var))
    }

    /// Create a new instance of an [expression][Exp] that denotes a [λ abstraction][Exp::Abs].
    pub fn new_abs(var: Var, typ: Exp, mut exp: Exp) -> Self {
        exp.index(&Idx::new(&var)); // set up de Bruijn indices
        Self::Abs(var, Box::new(typ), Box::new(exp))
    }

    /// Create a new instance of an [expression][Exp] that denotes a [Π type][Exp::For].
    pub fn new_for(var: Var, typ: Exp, mut exp: Exp) -> Self {
        exp.index(&Idx::new(&var)); // set up de Bruijn indices
        Self::For(var, Box::new(typ), Box::new(exp))
    }

    /// Create a new instance of an [expression][Exp] that denotes an [application][Exp::App].
    pub fn new_app(fst: Exp, snd: Exp) -> Self {
        Self::App(Box::new(fst), Box::new(snd))
    }

    /// Get the type of types (represented by `*`).
    pub fn get_type_meta() -> Self {
        Self::TypeMeta
    }

    /// Get the type of kinds (aka, universe, represented by `□`).
    pub fn get_kind_meta() -> Self {
        Self::KindMeta
    }

    /// Index an expression, converting bound variables into respective de Bruijn indices.
    pub fn index(&mut self, idx: &Idx) {
        if let Exp::Var(varidx) = self {
            if let VarIdx::Var(var) = varidx {
                if var == &idx.1 {
                    *varidx = VarIdx::new_idx(idx.clone())
                }
            } // update if binding variable matches
        } else if let Exp::Abs(var, _, exp) = self {
            if var != &idx.1 {
                exp.index(&idx.inc());
            } // short-circuit if binding variable is shadowed
        } else if let Exp::For(var, _, exp) = self {
            if var != &idx.1 {
                exp.index(&idx.inc());
            } // short-circuit if binding variable is shadowed
        } else if let Exp::App(fst, snd) = self {
            fst.index(idx);
            snd.index(idx);
        }
    }

    /// Reduce this expression to beta-normal form, or until the expression remains unchanged upon reduction.
    pub fn reduce(self, ctx: &Ctx) -> Result<Self, TypingErr> {
        let p = self.clone();
        let q = self.reduce_once(ctx)?;
        if p == q {
            Ok(q)
        } else {
            q.reduce_once(ctx)
        }
    }

    /// Calculate the type of this expression.
    pub fn calculate_type(&self, _ctx: &Ctx) -> Result<Exp, TypingErr> {
        match self {
            Exp::Var(_) => todo!(),
            Exp::Abs(_, _, _) => todo!(),
            Exp::For(_, _, _) => todo!(),
            Exp::App(_, _) => todo!(),
            Exp::TypeMeta => Ok(Exp::KindMeta), // SORT RULE
            Exp::KindMeta => Err(TypingErr::from(TypeUndefErr::new(self))), // not permitted
        }
    }

    /// Check that the type of this expression matches the given type.
    fn validate_type(&self, typ: &[&Exp], ctx: &Ctx) -> Result<(), TypingErr> {
        let act = &self.calculate_type(ctx)?;
        for t in typ {
            if act == *t {
                return Ok(());
            }
        }
        Err(TypingErr::from(TypeCompatErr::new(self, act, typ)))
    }

    /// Perform a one-step beta-reduction on this expression.
    fn reduce_once(self, ctx: &Ctx) -> Result<Self, TypingErr> {
        if let Exp::Abs(var, typ, exp) = self {
            return Ok(Exp::Abs(
                var,
                Box::new(typ.reduce(ctx)?),
                Box::new(exp.reduce(ctx)?),
            ));
        }
        if let Exp::For(var, typ, exp) = self {
            return Ok(Exp::For(
                var,
                Box::new(typ.reduce(ctx)?),
                Box::new(exp.reduce(ctx)?),
            ));
        }
        if let Exp::App(fst, snd) = self {
            if let Exp::Abs(var, typ, exp) = *fst {
                snd.validate_type(&[&typ], ctx)?;
                return Ok(exp.subst(&Idx::new(&var), &snd));
            } else {
                return Ok(Exp::App(
                    Box::new(fst.reduce(ctx)?),
                    Box::new(snd.reduce(ctx)?),
                ));
            }
        }
        Ok(self)
    }

    /// Replace all occurrences of the index with the given expression, in the current expression.
    fn subst(self, loc: &Idx, can: &Exp) -> Self {
        match self {
            Exp::Var(varidx) => match varidx {
                VarIdx::Var(var) => Exp::Var(VarIdx::Var(var)),
                VarIdx::Idx(idx) => match idx.cmp(loc) {
                    Ordering::Equal => can.clone(),
                    Ordering::Greater => Exp::Var(VarIdx::Idx(idx.dec())),
                    Ordering::Less => Exp::Var(VarIdx::Idx(idx)),
                },
            },
            Exp::Abs(var, typ, exp) => Exp::Abs(var, typ, Box::new(exp.subst(&loc.inc(), can))),
            Exp::For(var, typ, exp) => Exp::For(var, typ, Box::new(exp.subst(&loc.inc(), can))),
            Exp::App(fst, snd) => {
                Exp::App(Box::new(fst.subst(loc, can)), Box::new(snd.subst(loc, can)))
            }
            Exp::TypeMeta => self,
            Exp::KindMeta => self,
        }
    }

    /// Format this expression into canonical form.
    fn fmt(&self, f: &mut Formatter<'_>, flags: Branch) -> std::fmt::Result {
        match self {
            Self::Var(varidx) => varidx.fmt(f),
            Self::Abs(var, typ, exp) => Exp::fmt_binder(f, flags, "λ", var, typ, exp),
            Self::For(var, typ, exp) => Exp::fmt_binder(f, flags, "Π", var, typ, exp),
            Self::App(fst, snd) => Exp::fmt_app(f, flags, fst, snd),
            Self::TypeMeta => write!(f, "*"),
            Self::KindMeta => write!(f, "□"),
        }
    }

    /// Format a binder expression (λ abstraction or Π type).
    fn fmt_binder(
        f: &mut Formatter<'_>,
        flags: Branch,
        binder: &str,
        var: &Var,
        typ: &Exp,
        exp: &Exp,
    ) -> std::fmt::Result {
        let func = |f: &mut Formatter<'_>| -> std::fmt::Result {
            write!(f, "{}{} : ", binder, var)?;
            typ.fmt(f, Default::default())?; // reset, always greedy
            write!(f, " . ")?;
            exp.fmt(f, Default::default()) // reset, always greedy
        };
        Exp::parens(f, flags.ltree, func) // parenthesize if on the left side of tree
    }

    /// Format an application of one expression to another.
    fn fmt_app(f: &mut Formatter<'_>, flags: Branch, fst: &Exp, snd: &Exp) -> std::fmt::Result {
        let func = |f: &mut Formatter<'_>| -> std::fmt::Result {
            fst.fmt(
                f,
                Branch {
                    ltree: !flags.rtree, // true, but reset if current term is being parenthesized
                    rtree: flags.rtree,  // inherit from parent
                },
            )?;
            write!(f, " ")?;
            snd.fmt(
                f,
                Branch {
                    ltree: flags.ltree,  // inherit from parent
                    rtree: !flags.rtree, // true, but reset if current term is being parenthesized
                },
            )
        };
        Exp::parens(f, flags.rtree, func) // parenthesize if on the right side of tree
    }

    /// Parenthesize (or not) as specified, executing a closure to write the content within.
    fn parens<F>(f: &mut Formatter<'_>, parens: bool, func: F) -> std::fmt::Result
    where
        F: FnOnce(&mut Formatter<'_>) -> std::fmt::Result,
    {
        if parens {
            write!(f, "(")?;
        }
        func(f)?;
        if parens {
            write!(f, ")")?;
        }
        Ok(())
    }
}

impl Display for Exp {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.fmt(f, Branch::new())
    }
}

impl Branch {
    /// Create a new instance of branch.
    pub fn new() -> Self {
        Branch {
            ltree: false,
            rtree: false,
        }
    }
}

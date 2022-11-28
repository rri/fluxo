//! Typing context and related utilities.

use crate::ast::{Exp, Var};
use crate::err::{TypeRedeclErr, TypeUnknownErr};
use std::collections::HashMap;

/// Typing context, usually represented with the symbol 'Γ'.
#[derive(Debug, Default, Clone)]
pub struct Ctx {
    map: HashMap<Var, Exp>,
}

impl Ctx {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
}

impl Ctx {
    /// Register a variable and its associated type in this typing context.
    pub fn put(&mut self, var: &Var, typ: &Exp) -> Result<(), TypeRedeclErr> {
        let old = self.map.get(var);
        if let Some(old) = old {
            if old != typ {
                return Err(TypeRedeclErr::new(var, old, typ));
            }
        }
        self.map.insert(var.clone(), typ.clone());
        Ok(())
    }

    /// Fetch the type associated with a variable in this typing context.
    pub fn get(&self, var: &Var) -> Result<&Exp, TypeUnknownErr> {
        self.map
            .get(var)
            .map_or_else(|| Err(TypeUnknownErr::new(var)), Ok)
    }

    /// Extend this context with a variable and return the context, without modifying the original.
    pub fn extend(&self, var: &Var, typ: &Exp) -> Result<Ctx, TypeRedeclErr> {
        let mut can = self.clone();
        can.put(var, typ)?;
        Ok(can)
    }

    /// Return a new context without the given variable, without modifying the original.
    pub fn remove(&self, var: &Var) -> Result<Ctx, TypeUnknownErr> {
        let mut can = self.clone();
        can.map
            .remove(var)
            .map_or_else(|| Err(TypeUnknownErr::new(var)), |_| Ok(can))
    }
}

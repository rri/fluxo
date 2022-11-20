//! Typing context and related utilities.

use super::{Exp, Var};
use crate::err::{TypeRedeclErr, TypeUnknownErr};
use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct Ctx {
    map: HashMap<Var, Exp>,
}

impl Ctx {
    pub fn new() -> Self {
        Ctx {
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

    /// Extend this context with a variable and return the context, without modifying the original.
    pub fn extend(&self, var: &Var, typ: &Exp) -> Result<Ctx, TypeRedeclErr> {
        let mut can = self.clone();
        can.put(var, typ)?;
        Ok(can)
    }

    /// Fetch the type associated with a variable in this typing context.
    pub fn get(&self, var: &Var) -> Result<&Exp, TypeUnknownErr> {
        self.map
            .get(var)
            .map_or_else(|| Err(TypeUnknownErr::new(var)), Ok)
    }
}

mod compile;
pub mod game;
mod runtime;
pub mod stdlib;

pub use compile::{ast, bytecode, compiler, lexer, parser};
pub use runtime::{exec, heap, meta, number, table, value, vm};

#[cfg(test)]
mod tests;

use std::rc::Rc;

pub use value::{GcRef, Str, Value};
pub use vm::{Lua, LuaError, LuaResult};

impl Lua {
    pub fn load(&mut self, src: &[u8], chunkname: &str) -> LuaResult<Value> {
        let body = parser::parse(src, chunkname).map_err(|e| LuaError::Error(Value::str(&e.0)))?;
        let proto = compiler::compile(&body, Rc::from(chunkname));
        let g = self.globals;
        Ok(self.new_closure(proto, Vec::new(), g))
    }

    pub fn exec(&mut self, src: &[u8], chunkname: &str) -> LuaResult<Vec<Value>> {
        let f = self.load(src, chunkname)?;
        self.call(f, &[])
    }
}

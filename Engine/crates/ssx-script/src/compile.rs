use mlua::chunk::Compiler;

use crate::text::display_bounded;

pub const COMPILE_STACK: usize = 16 << 20;
const MAX_MESSAGE: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileError(pub String);

pub fn looks_like_bytecode(data: &[u8]) -> bool {
    match data {
        [] => false,
        [b, ..] if *b < b'\t' => true,
        [b, rest @ ..] if *b <= 11 => matches!(rest.first(), Some(1..=3)),
        _ => false,
    }
}

pub fn bytecode(name: &str, source: &[u8]) -> Result<Vec<u8>, CompileError> {
    if looks_like_bytecode(source) {
        return Err(CompileError("attempt to load a binary chunk (mode is 't')".into()));
    }
    let (compiled, cpu) = std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("luau-compile".into())
            .stack_size(COMPILE_STACK)
            .spawn_scoped(scope, || {
                let start = crate::cpu::thread_ns();
                let compiled = Compiler::new().compile(source);
                let used = start.zip(crate::cpu::thread_ns()).map_or(0, |(a, b)| b.saturating_sub(a));
                (compiled, used)
            })
            .map_err(|e| CompileError(format!("the compiler's thread could not start: {e}")))?;
        worker.join().map_err(|_| CompileError("the compiler's thread failed".into()))
    })?;
    crate::cpu::add_waited(cpu);
    compiled.map_err(|e| match e {
        mlua::Error::SyntaxError { message, .. } => {
            CompileError(format!("{name}:{}", crate::env::cap(&message, MAX_MESSAGE)))
        }
        other => CompileError(display_bounded(&other, MAX_MESSAGE)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_carry_the_name_and_line() {
        let e = bytecode("pkg/a.luau", b"local x = \n\n)").unwrap_err();
        assert!(e.0.starts_with("pkg/a.luau:3:"), "{}", e.0);
        assert!(bytecode("x", b"\x01binary").unwrap_err().0.contains("binary chunk"));
        assert!(bytecode("x", b"return 1").is_ok_and(|b| !b.is_empty()));
    }

    #[test]
    fn bytecode_is_told_apart_as_mlua_tells_it() {
        for a in 0..=255u8 {
            for b in 0..=255u8 {
                let data = [a, b];
                let theirs = unsafe { mlua::ffi::luaL_isbytecode(data.as_ptr().cast(), data.len()) };
                assert_eq!(looks_like_bytecode(&data), theirs, "{a} {b}");
            }
            let theirs = unsafe { mlua::ffi::luaL_isbytecode([a].as_ptr().cast(), 1) };
            assert_eq!(looks_like_bytecode(&[a]), theirs, "{a}");
        }
        assert!(!looks_like_bytecode(b""));
    }

    #[test]
    fn the_deepest_accepted_source_compiles_off_a_small_caller_stack() {
        let src = format!("local x = {{}} return x{}", ":m()".repeat(998));
        let r = std::thread::Builder::new().stack_size(256 << 10).spawn(move || bytecode("deep", src.as_bytes()));
        assert!(r.unwrap().join().unwrap().is_ok());
    }
}

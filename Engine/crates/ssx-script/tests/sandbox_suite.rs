#[path = "suite/cases.rs"]
mod cases;

use cases::{cases, check, run};

fn case(name: &str) {
    let all = cases();
    let c = all.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no case {name}"));
    let r = run(c);
    if let Err(e) = check(c, &r) {
        panic!("premise: {}\n{e}", c.premise);
    }
}

macro_rules! suite {
    ($($name:ident),* $(,)?) => {
        $( #[test] fn $name() { case(stringify!($name)); } )*
        const NAMES: &[&str] = &[$(stringify!($name)),*];
    };
}

suite!(
    while_true,
    while_true_running,
    tail_recursion,
    deep_recursion,
    string_rep_loop,
    gsub_loop,
    coroutine_loop,
    pcall_resumes,
    xpcall_resumes,
    coroutine_catches,
    handler_loops,
    timer_loops,
    tick_budget,
    straight_line_body,
    c_call_loop,
    gc_churn,
    table_growth,
    concat_bomb,
    buffer_bomb,
    rep_bomb,
    closure_bomb,
    server_heap,
    no_debug,
    no_io_os,
    no_fenv,
    loadstring_stays_inside,
    no_bytecode,
    require_up,
    require_absolute,
    require_case,
    case_collision,
    readonly_globals,
    api_tables_readonly,
    event_flood,
    timer_flood,
    data_key_flood,
    data_byte_flood,
    element_flood,
    outbox_flood,
    latent_flood,
    value_depth,
    value_long_string,
    value_cycle,
    value_shared_subtable,
    value_metatable,
    value_function,
    deep_nesting,
    million_terms,
    long_chain,
    huge_literal,
    deep_method_chain,
    loadstring_deep,
    chain_export_loop,
    chain_export_pcall,
    chain_export_loop_server,
    chain_event_loop,
    chain_depth_cap,
    chain_middle,
    chain_callee_runaway,
    chain_tick_charge,
    leftovers,
);

#[test]
fn every_case_has_its_test() {
    let mut listed: Vec<&str> = NAMES.to_vec();
    listed.sort_unstable();
    let all: Vec<&str> = cases().iter().map(|c| c.name).collect();
    assert_eq!(listed, all);
}

#[cfg(unix)]
#[test]
fn require_of_a_linked_file_finds_nothing() {
    use ssx_script::host::package::PackageScripts;
    use ssx_script::host::{HostConfig, ScriptHost, Side};
    let dir = std::env::temp_dir().join(format!("ssx-script-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("client")).unwrap();
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(
        dir.join("resource.toml"),
        "[resource]\nname = \"linked\"\nversion = \"1.0.0\"\napi = 1\n[scripts]\nclient = [\"client/*.luau\"]\n[files]\ninclude = [\"lib/*\"]\n",
    )
    .unwrap();
    std::fs::write(dir.join("client/main.luau"), "print(pcall(require, 'lib/secret'))").unwrap();
    let outside = dir.with_extension("outside.luau");
    std::fs::write(&outside, "return 'leaked'").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("lib/secret.luau")).unwrap();
    let src = ssx_content::PackageSource::open(&dir).unwrap();
    let host = ScriptHost::new(HostConfig::new(Side::Client)).unwrap();
    let pkg = PackageScripts::from_source(&src, Side::Client, host.limits()).unwrap();
    assert!(!pkg.modules.contains_key("lib/secret.luau"), "the link is not a module");
    host.start(pkg).unwrap();
    let lines = host.take_output("linked").unwrap().lines;
    assert!(lines[0].starts_with("false\t") && lines[0].contains("no such module"), "{lines:?}");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&outside);
}

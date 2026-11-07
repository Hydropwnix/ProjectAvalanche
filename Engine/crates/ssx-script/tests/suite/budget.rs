use super::*;

const SPIN: &str = "local function spin() local x = 0 for i = 1, 8000 do x += i end return x end\n";

fn other(name: &'static str, side: Side, src: String) -> Other {
    (name, vec![(main_file(side), src)])
}

fn spinner(side: Side) -> Vec<Other> {
    vec![other("b", side, format!("{SPIN}exports('spin', function() spin() return 1 end)"))]
}

const LINKS: [&str; 7] = ["p2", "p3", "p4", "p5", "p6", "p7", "p8"];

fn links() -> Vec<Other> {
    let mut v: Vec<Other> = (2..9)
        .map(|i| other(LINKS[i - 2], C, format!("exports('f', function() return exports.p{}:f() end)", i + 1)))
        .collect();
    v.push(other("p9", C, format!("{SPIN}exports('f', function() spin() return 1 end)")));
    v
}

pub(super) fn cases() -> Vec<Case> {
    vec![
        budget_case(
            "while_true",
            "an endless loop at a script's top level stops it before it starts",
            C,
            || client("while true do end"),
            Expect::StartFails(BUDGET),
        ),
        budget_running(
            "while_true_running",
            "an endless loop in a running package is stopped by the call budget",
            C,
            || later(C, "while true do end"),
            Expect::Stopped(BUDGET),
        ),
        running(
            "tail_recursion",
            "an endless tail call overflows Luau's stack (Luau has no tail calls): a script error, the package goes on",
            C,
            || later(C, "local function f() return f() end return f()"),
            Expect::Prints("stack overflow"),
        ),
        case(
            "deep_recursion",
            "unbounded recursion ends as Luau's stack overflow error, never a crash",
            C,
            || client("local function f(n) return 1 + f(n + 1) end f(1)"),
            Expect::StartFails("stack overflow"),
        ),
        budget_running(
            "string_rep_loop",
            "string.rep in a loop is stopped by the call budget: about 350 ns a checkpoint on the fastest core measured, \
             where the count (250 000) and the backstop (80 ms) come close, so either may be first",
            C,
            || later(C, "local s = '' while true do s = string.rep('ab', 1000) .. s:sub(1, 10) end"),
            Expect::Stopped(BUDGET),
        ),
        budget_running(
            "gsub_loop",
            "string.gsub in a loop reaches the interrupt inside the matcher and between calls",
            C,
            || later(C, "local s = string.rep('a', 4000) while true do s = s:gsub('a', 'a') end"),
            Expect::Stopped(BUDGET),
        ),
        budget_running(
            "coroutine_loop",
            "coroutines resumed in a loop are stopped",
            C,
            || {
                later(
                    C,
                    "local co = coroutine.wrap(function() while true do coroutine.yield() end end) while true do co() end",
                )
            },
            Expect::Stopped(BUDGET),
        ),
        budget_running(
            "pcall_resumes",
            "the budget's error caught by pcall and the loop resumed: stopped all the same",
            C,
            || later(C, "while true do pcall(function() while true do end end) end"),
            Expect::Stopped(BUDGET),
        ),
        budget_running(
            "xpcall_resumes",
            "the budget's error caught by xpcall and the loop resumed: stopped all the same",
            C,
            || later(C, "while true do xpcall(function() while true do end end, function(e) return e end) end"),
            Expect::Stopped(BUDGET),
        ),
        budget_running(
            "coroutine_catches",
            "the budget's error caught by coroutine.resume and the loop resumed: stopped all the same",
            C,
            || later(C, "while true do coroutine.resume(coroutine.create(function() while true do end end)) end"),
            Expect::Stopped(BUDGET),
        ),
        budget_running(
            "handler_loops",
            "an error handler that loops: stopped all the same",
            C,
            || later(C, "xpcall(function() error('x') end, function() while true do end end) while true do end"),
            Expect::Stopped(BUDGET),
        ),
        Case {
            ticks: 2,
            ..budget_case(
                "timer_loops",
                "a timer that never returns is stopped, named in the reason",
                C,
                || client("setTimer(function() while true do end end, 50, 1)"),
                Expect::Stopped("timer 2 (client/main.luau:1)"),
            )
        },
        Case {
            ticks: 35,
            limits: Lim::TickOnly,
            ..case(
                "tick_budget",
                "a package that alone outruns the tick budget 30 ticks in a row is stopped",
                C,
                || client("setTimer(function() local x = 0 for i = 1, 100000 do x += i end end, 50, 0)"),
                Expect::Stopped("the tick budget of 50000 checkpoints"),
            )
        },
        budget_running(
            "straight_line_body",
            "a loop whose body is 20 000 statements (one checkpoint an iteration) is stopped by the backstop",
            C,
            || {
                vec![(
                    main_file(C),
                    format!(
                        "setTimer(function() local x = 0 while true do {} end end, 50, 1)",
                        "x += 1 ".repeat(20_000)
                    ),
                )]
            },
            Expect::Stopped(BACKSTOP),
        ),
        budget_running(
            "c_call_loop",
            "a loop around one long C call (table.sort of 100 000 numbers, no checkpoint inside) is stopped by the backstop",
            C,
            || {
                later(
                    C,
                    "local t = {} for i = 1, 100000 do t[i] = (i * 7919) % 100003 end while true do table.sort(t) end",
                )
            },
            Expect::Stopped(BACKSTOP),
        ),
        budget_running(
            "gc_churn",
            "50 000 new strings an iteration from one C call (string.split), the collector's steps inside it, is stopped by the backstop",
            C,
            || {
                client(
                    "local p = {} for i = 1, 50000 do p[i] = tostring(i) end local s = table.concat(p, ',') p = nil \
                     setTimer(function() while true do local parts = string.split(s, ',') end end, 50, 1)",
                )
            },
            Expect::Stopped(BACKSTOP),
        ),
        Case {
            others: Some(|| spinner(C)),
            ..budget_running(
                "chain_export_loop",
                "a package that loops on another's export, each call short, is stopped at the chain bound; the callee goes on",
                C,
                || later(C, "while true do exports.b:spin() end"),
                Expect::Stopped(CHAIN),
            )
        },
        Case {
            others: Some(|| spinner(C)),
            ..budget_running(
                "chain_export_pcall",
                "catching every failed call does not keep a looping caller past the chain bound",
                C,
                || later(C, "while true do pcall(function() return exports.b:spin() end) end"),
                Expect::Stopped(CHAIN),
            )
        },
        Case {
            others: Some(|| spinner(S)),
            ..budget_running(
                "chain_export_loop_server",
                "the server's chain bound (2 500 000 checkpoints) stops an export loop the same way",
                S,
                || later(S, "while true do exports.b:spin() end"),
                Expect::Stopped(CHAIN),
            )
        },
        Case {
            others: Some(|| {
                vec![other(
                    "b",
                    C,
                    format!("{SPIN}addEvent('ping') addEventHandler('ping', root, function() spin() end)"),
                )]
            }),
            ..budget_running(
                "chain_event_loop",
                "a package that loops on an event another package handles is stopped at the chain bound",
                C,
                || later(C, "while true do triggerEvent('ping', root) end"),
                Expect::Stopped(CHAIN),
            )
        },
        Case {
            others: Some(links),
            ..budget_running(
                "chain_depth_cap",
                "a loop over a chain of exports at the depth cap, each link under budget, stops the loop's package",
                C,
                || later(C, "while true do exports.p2:f() end"),
                Expect::Stopped(CHAIN),
            )
        },
        Case {
            others: Some(|| {
                vec![
                    other("head", C, "setTimer(function() exports.hostile:f() end, 50, 1)".to_string()),
                    other("tail", C, format!("{SPIN}exports('f', function() spin() return 1 end)")),
                ]
            }),
            ..budget_running(
                "chain_middle",
                "a package in the middle of a chain that loops on its callee is stopped; its caller and callee go on",
                C,
                || client("exports('f', function() while true do exports.tail:f() end end)"),
                Expect::Stopped(CHAIN),
            )
        },
        Case {
            ticks: 1,
            others: Some(|| {
                vec![other(
                    "head",
                    C,
                    "setTimer(function() print(pcall(function() return exports.hostile:spin() end)) end, 50, 1)"
                        .to_string(),
                )]
            }),
            ..budget_case(
                "chain_callee_runaway",
                "a callee's own runaway is stopped by its own call budget, before the chain bound; the caller goes on",
                C,
                || client("exports('spin', function() while true do end end)"),
                Expect::Stopped(BUDGET),
            )
        },
        Case {
            ticks: 35,
            limits: Lim::TickOnly,
            others: Some(|| spinner(C)),
            ..case(
                "chain_tick_charge",
                "a package that keeps another busy every tick, under the chain bound, is the one the tick budget stops",
                C,
                || client("setTimer(function() for i = 1, 10 do exports.b:spin() end end, 50, 0)"),
                Expect::Stopped("its calls used more than the tick budget of 50000 checkpoints"),
            )
        },
    ]
}

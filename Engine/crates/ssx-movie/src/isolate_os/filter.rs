use std::ffi::c_int;

use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen as Len, SeccompCmpOp as Op, SeccompCondition as Cond, SeccompFilter,
    SeccompRule, TargetArch,
};

pub struct Call {
    pub nr: i64,
    pub name: &'static str,
    pub why: &'static str,
}

// TODO(guess): the list is complete for every glibc a player has. Measured over the same inputs on glibc 2.39 (Ubuntu
// 24.04, the floor: the release's Linux build is made there), 2.41 (Debian 13), 2.43 (Fedora 44) and 2.44, all on
// Linux 7.3: no call outside it. A glibc outside these may reach the same work through other calls — the four already
// differ: only 2.43 and 2.44 make the detach's shared wait, only 2.39's abort unblocks SIGABRT, and only 2.43 and 2.44
// place a thread's guard page with madvise 102 (refused; their threads start before the seal). A call outside the
// list ends the movie with the call named, never the game. Settled by running sandbox_measure over the proof inputs
// on that glibc.
pub const CALLS: &[Call] = &[
    Call { nr: libc::SYS_read, name: "read", why: "the units from the parent: fd 0 only" },
    Call {
        nr: libc::SYS_write,
        name: "write",
        why: "the pictures to the parent (fd 1); a panic's or the reporter's text (fd 2)",
    },
    Call { nr: libc::SYS_writev, name: "writev", why: "glibc's report of a corrupt heap before its abort (fd 2 only)" },
    Call {
        nr: libc::SYS_mmap,
        name: "mmap",
        why: "the allocator's large blocks and arenas, a thread's signal stack: private, anonymous, never executable",
    },
    Call {
        nr: libc::SYS_mprotect,
        name: "mprotect",
        why: "an arena grows; a signal stack's guard page: never executable",
    },
    Call {
        nr: libc::SYS_munmap,
        name: "munmap",
        why: "the allocator gives large blocks back; a thread's signal stack at its end",
    },
    Call { nr: libc::SYS_brk, name: "brk", why: "the main heap" },
    Call { nr: libc::SYS_madvise, name: "madvise", why: "a thread's stack released at its end (MADV_DONTNEED only)" },
    Call {
        nr: libc::SYS_futex,
        name: "futex",
        why: "the locks and condition variables between rav1d's threads (private FUTEX_WAIT, FUTEX_WAKE, FUTEX_WAIT_BITSET); glibc 2.43's and 2.44's pthread_detach of a thread that is already ending (the decoder's last reference dropped on a worker after the close) joins it, waiting on its thread id: a shared wait, the kernel's wake (2.39 and 2.41 free the thread without a wait)",
    },
    Call { nr: libc::SYS_sched_yield, name: "sched_yield", why: "a lock spins before it waits" },
    Call {
        nr: libc::SYS_rt_sigprocmask,
        name: "rt_sigprocmask",
        why: "a thread's start sets its signal mask; glibc 2.39's abort() unblocks SIGABRT (2.41 and later do not)",
    },
    Call {
        nr: libc::SYS_sigaltstack,
        name: "sigaltstack",
        why: "a thread's start and end: Rust's stack-overflow signal stack",
    },
    Call { nr: libc::SYS_rseq, name: "rseq", why: "a thread's start: glibc registers it (flags 0)" },
    Call { nr: libc::SYS_set_robust_list, name: "set_robust_list", why: "a thread's start: glibc's robust lock list" },
    Call { nr: libc::SYS_prctl, name: "prctl", why: "a thread's start: its name (PR_SET_NAME only)" },
    Call {
        nr: libc::SYS_sched_getaffinity,
        name: "sched_getaffinity",
        why: "a thread's start: its own processor set (this process's threads only)",
    },
    Call { nr: libc::SYS_gettid, name: "gettid", why: "a thread's start; raise() in abort() and the reporter" },
    Call { nr: libc::SYS_getpid, name: "getpid", why: "raise() in abort() and the reporter" },
    Call { nr: libc::SYS_tgkill, name: "tgkill", why: "raise(): this process only, SIGABRT or SIGSYS" },
    Call { nr: libc::SYS_exit, name: "exit", why: "rav1d's threads end once the decoder is closed" },
    Call { nr: libc::SYS_exit_group, name: "exit_group", why: "the child's end" },
];

pub fn call_name(nr: i64) -> Option<&'static str> {
    NAMES.split(' ').find_map(|e| {
        let (n, name) = e.split_once(':')?;
        (n.parse::<i64>().ok()? == nr).then_some(name)
    })
}

const NAMES: &str = "0:read 1:write 2:open 3:close 4:stat 5:fstat 6:lstat 7:poll 8:lseek 9:mmap 10:mprotect 11:munmap 12:brk \
     13:rt_sigaction 14:rt_sigprocmask 15:rt_sigreturn 16:ioctl 17:pread64 18:pwrite64 19:readv 20:writev 21:access \
     22:pipe 23:select 24:sched_yield 25:mremap 26:msync 27:mincore 28:madvise 29:shmget 30:shmat 31:shmctl 32:dup \
     33:dup2 34:pause 35:nanosleep 36:getitimer 37:alarm 38:setitimer 39:getpid 40:sendfile 41:socket 42:connect \
     43:accept 44:sendto 45:recvfrom 46:sendmsg 47:recvmsg 48:shutdown 49:bind 50:listen 51:getsockname \
     52:getpeername 53:socketpair 54:setsockopt 55:getsockopt 56:clone 57:fork 58:vfork 59:execve 60:exit 61:wait4 \
     62:kill 63:uname 64:semget 65:semop 66:semctl 67:shmdt 68:msgget 69:msgsnd 70:msgrcv 71:msgctl 72:fcntl \
     73:flock 74:fsync 75:fdatasync 76:truncate 77:ftruncate 78:getdents 79:getcwd 80:chdir 81:fchdir 82:rename \
     83:mkdir 84:rmdir 85:creat 86:link 87:unlink 88:symlink 89:readlink 90:chmod 91:fchmod 92:chown 93:fchown \
     94:lchown 95:umask 96:gettimeofday 97:getrlimit 98:getrusage 99:sysinfo 100:times 101:ptrace 102:getuid \
     103:syslog 104:getgid 105:setuid 106:setgid 107:geteuid 108:getegid 109:setpgid 110:getppid 111:getpgrp \
     112:setsid 113:setreuid 114:setregid 115:getgroups 116:setgroups 117:setresuid 118:getresuid 119:setresgid \
     120:getresgid 121:getpgid 122:setfsuid 123:setfsgid 124:getsid 125:capget 126:capset 127:rt_sigpending \
     128:rt_sigtimedwait 129:rt_sigqueueinfo 130:rt_sigsuspend 131:sigaltstack 132:utime 133:mknod 134:uselib \
     135:personality 136:ustat 137:statfs 138:fstatfs 139:sysfs 140:getpriority 141:setpriority 142:sched_setparam \
     143:sched_getparam 144:sched_setscheduler 145:sched_getscheduler 146:sched_get_priority_max \
     147:sched_get_priority_min 148:sched_rr_get_interval 149:mlock 150:munlock 151:mlockall 152:munlockall \
     153:vhangup 154:modify_ldt 155:pivot_root 156:_sysctl 157:prctl 158:arch_prctl 159:adjtimex 160:setrlimit \
     161:chroot 162:sync 163:acct 164:settimeofday 165:mount 166:umount2 167:swapon 168:swapoff 169:reboot \
     170:sethostname 171:setdomainname 172:iopl 173:ioperm 175:init_module 176:delete_module 179:quotactl \
     180:nfsservctl 181:getpmsg 182:putpmsg 183:afs_syscall 184:tuxcall 185:security 186:gettid 187:readahead \
     188:setxattr 189:lsetxattr 190:fsetxattr 191:getxattr 192:lgetxattr 193:fgetxattr 194:listxattr 195:llistxattr \
     196:flistxattr 197:removexattr 198:lremovexattr 199:fremovexattr 200:tkill 201:time 202:futex \
     203:sched_setaffinity 204:sched_getaffinity 205:set_thread_area 206:io_setup 207:io_destroy 208:io_getevents \
     209:io_submit 210:io_cancel 211:get_thread_area 212:lookup_dcookie 213:epoll_create 214:epoll_ctl_old \
     215:epoll_wait_old 216:remap_file_pages 217:getdents64 218:set_tid_address 219:restart_syscall 220:semtimedop \
     221:fadvise64 222:timer_create 223:timer_settime 224:timer_gettime 225:timer_getoverrun 226:timer_delete \
     227:clock_settime 228:clock_gettime 229:clock_getres 230:clock_nanosleep 231:exit_group 232:epoll_wait \
     233:epoll_ctl 234:tgkill 235:utimes 236:vserver 237:mbind 238:set_mempolicy 239:get_mempolicy 240:mq_open \
     241:mq_unlink 242:mq_timedsend 243:mq_timedreceive 244:mq_notify 245:mq_getsetattr 246:kexec_load 247:waitid \
     248:add_key 249:request_key 250:keyctl 251:ioprio_set 252:ioprio_get 253:inotify_init 254:inotify_add_watch \
     255:inotify_rm_watch 256:migrate_pages 257:openat 258:mkdirat 259:mknodat 260:fchownat 261:futimesat \
     262:newfstatat 263:unlinkat 264:renameat 265:linkat 266:symlinkat 267:readlinkat 268:fchmodat 269:faccessat \
     270:pselect6 271:ppoll 272:unshare 273:set_robust_list 274:get_robust_list 275:splice 276:tee \
     277:sync_file_range 278:vmsplice 279:move_pages 280:utimensat 281:epoll_pwait 282:signalfd 283:timerfd_create \
     284:eventfd 285:fallocate 286:timerfd_settime 287:timerfd_gettime 288:accept4 289:signalfd4 290:eventfd2 \
     291:epoll_create1 292:dup3 293:pipe2 294:inotify_init1 295:preadv 296:pwritev 297:rt_tgsigqueueinfo \
     298:perf_event_open 299:recvmmsg 300:fanotify_init 301:fanotify_mark 302:prlimit64 303:name_to_handle_at \
     304:open_by_handle_at 305:clock_adjtime 306:syncfs 307:sendmmsg 308:setns 309:getcpu 310:process_vm_readv \
     311:process_vm_writev 312:kcmp 313:finit_module 314:sched_setattr 315:sched_getattr 316:renameat2 317:seccomp \
     318:getrandom 319:memfd_create 320:kexec_file_load 321:bpf 322:execveat 323:userfaultfd 324:membarrier \
     325:mlock2 326:copy_file_range 327:preadv2 328:pwritev2 329:pkey_mprotect 330:pkey_alloc 331:pkey_free \
     332:statx 334:rseq 424:pidfd_send_signal 425:io_uring_setup 426:io_uring_enter 427:io_uring_register \
     428:open_tree 429:move_mount 430:fsopen 431:fsconfig 432:fsmount 433:fspick 434:pidfd_open 435:clone3 \
     436:close_range 437:openat2 438:pidfd_getfd 439:faccessat2 440:process_madvise 441:epoll_pwait2 \
     442:mount_setattr 443:quotactl_fd 444:landlock_create_ruleset 445:landlock_add_rule 446:landlock_restrict_self \
     447:memfd_secret 448:process_mrelease 449:futex_waitv 450:set_mempolicy_home_node 452:fchmodat2 462:mseal";

fn cond(arg: u8, len: Len, op: Op, value: u64) -> Result<Cond, String> {
    Cond::new(arg, len, op, value).map_err(|e| format!("the filter's condition: {e}"))
}

fn rule(conds: Vec<Cond>) -> Result<SeccompRule, String> {
    SeccompRule::new(conds).map_err(|e| format!("the filter's rule: {e}"))
}

fn only(arg: u8, allowed: u64) -> Result<Cond, String> {
    cond(arg, Len::Qword, Op::MaskedEq(!allowed), 0)
}

fn rules(nr: i64, pid: i32, tids: &[i32]) -> Result<Vec<SeccompRule>, String> {
    let eq = |arg: u8, v: u64| cond(arg, Len::Dword, Op::Eq, v);
    let one = |arg: u8, v: u64| -> Result<SeccompRule, String> { rule(vec![eq(arg, v)?]) };
    let rw = (libc::PROT_READ | libc::PROT_WRITE) as u64;
    Ok(match nr {
        libc::SYS_read => vec![one(0, 0)?],
        libc::SYS_write => vec![one(0, 1)?, one(0, 2)?],
        libc::SYS_writev => vec![one(0, 2)?],
        libc::SYS_mmap => {
            let private = (libc::MAP_PRIVATE | libc::MAP_ANONYMOUS) as u64;
            vec![rule(vec![
                only(2, rw)?,
                only(3, private | libc::MAP_STACK as u64)?,
                cond(3, Len::Qword, Op::MaskedEq(private), private)?,
            ])?]
        }
        libc::SYS_mprotect => vec![rule(vec![only(2, rw)?])?],
        libc::SYS_madvise => vec![one(2, libc::MADV_DONTNEED as u64)?],
        libc::SYS_futex => {
            let private = libc::FUTEX_PRIVATE_FLAG;
            let realtime = libc::FUTEX_CLOCK_REALTIME;
            let ops = [libc::FUTEX_WAIT, libc::FUTEX_WAKE, libc::FUTEX_WAIT_BITSET];
            let mut r = ops.iter().map(|&op| one(1, (op | private) as u64)).collect::<Result<Vec<_>, _>>()?;
            r.push(one(1, (libc::FUTEX_WAIT_BITSET | realtime) as u64)?);
            r
        }
        libc::SYS_prctl => vec![one(0, libc::PR_SET_NAME as u64)?],
        libc::SYS_rseq => vec![one(2, 0)?],
        libc::SYS_sched_getaffinity => tids.iter().map(|&t| one(0, t as u32 as u64)).collect::<Result<_, _>>()?,
        libc::SYS_tgkill => {
            let to = |sig: c_int| -> Result<SeccompRule, String> {
                rule(vec![eq(0, pid as u32 as u64)?, eq(2, sig as u64)?])
            };
            vec![to(libc::SIGABRT)?, to(libc::SIGSYS)?]
        }
        _ => Vec::new(),
    })
}

pub fn program(pid: i32, tids: &[i32], listed: bool) -> Result<BpfProgram, String> {
    let mut map = std::collections::BTreeMap::new();
    if listed {
        for c in CALLS {
            map.insert(c.nr, rules(c.nr, pid, tids)?);
        }
    }
    let f = SeccompFilter::new(map, SeccompAction::Trap, SeccompAction::Allow, TargetArch::x86_64)
        .map_err(|e| format!("the filter: {e}"))?;
    BpfProgram::try_from(f).map_err(|e| format!("the filter: {e}"))
}

pub(crate) fn enforce(pid: i32, tids: &[i32]) -> Result<(), String> {
    let p = program(pid, tids, true)?;
    seccompiler::apply_filter_all_threads(&p).map_err(|e| format!("the seccomp filter: {e}"))
}

pub(crate) fn measure() -> Result<(), String> {
    let pid = unsafe { libc::getpid() };
    let mut p = program(pid, &[], false)?;
    for i in &mut p {
        if i.code == (BPF_RET | BPF_K) && i.k == libc::SECCOMP_RET_TRAP {
            i.k = SECCOMP_RET_USER_NOTIF;
        }
    }
    let prog = libc::sock_fprog {
        len: u16::try_from(p.len()).map_err(|_| "the filter's length")?,
        filter: p.as_mut_ptr().cast::<libc::sock_filter>(),
    };
    let flags = libc::SECCOMP_FILTER_FLAG_TSYNC
        | libc::SECCOMP_FILTER_FLAG_TSYNC_ESRCH
        | libc::SECCOMP_FILTER_FLAG_NEW_LISTENER;
    let fd = unsafe { libc::syscall(libc::SYS_seccomp, libc::SECCOMP_SET_MODE_FILTER, flags, &raw const prog) };
    if fd != 3 {
        return Err(format!("the measurement's filter: {fd} ({})", std::io::Error::last_os_error()));
    }
    marker(true)
}

pub(crate) fn marker(dumpable: bool) -> Result<(), String> {
    let v = libc::c_ulong::from(dumpable);
    let z: libc::c_ulong = 0;
    if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, v, z, z, z) } != 0 {
        return Err(format!("PR_SET_DUMPABLE: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

const BPF_LD: u16 = 0x00;
const BPF_ALU: u16 = 0x04;
const BPF_JMP: u16 = 0x05;
const BPF_RET: u16 = 0x06;
const BPF_W: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_AND: u16 = 0x50;
const BPF_JA: u16 = 0x00;
const BPF_JEQ: u16 = 0x10;
const BPF_JGT: u16 = 0x20;
const BPF_JGE: u16 = 0x30;
const BPF_K: u16 = 0x00;
pub const SECCOMP_RET_USER_NOTIF: u32 = 0x7fc0_0000;

pub fn evaluate(p: &[seccompiler::sock_filter], nr: i32, arch: u32, args: [u64; 6]) -> Result<u32, String> {
    let mut data = [0u8; 64];
    data[..4].copy_from_slice(&nr.to_le_bytes());
    data[4..8].copy_from_slice(&arch.to_le_bytes());
    for (k, a) in args.iter().enumerate() {
        data[16 + 8 * k..24 + 8 * k].copy_from_slice(&a.to_le_bytes());
    }
    let (mut pc, mut acc) = (0usize, 0u32);
    for _ in 0..=p.len() {
        let i = p.get(pc).ok_or("the program runs past its end")?;
        pc += 1;
        let jump = |taken: bool| usize::from(if taken { i.jt } else { i.jf });
        match i.code {
            c if c == BPF_LD | BPF_W | BPF_ABS => {
                let at = i.k as usize;
                let w = data.get(at..at + 4).filter(|_| at.is_multiple_of(4)).ok_or("a load outside seccomp_data")?;
                acc = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
            }
            c if c == BPF_ALU | BPF_AND | BPF_K => acc &= i.k,
            c if c == BPF_JMP | BPF_JA => pc += i.k as usize,
            c if c == BPF_JMP | BPF_JEQ | BPF_K => pc += jump(acc == i.k),
            c if c == BPF_JMP | BPF_JGT | BPF_K => pc += jump(acc > i.k),
            c if c == BPF_JMP | BPF_JGE | BPF_K => pc += jump(acc >= i.k),
            c if c == BPF_RET | BPF_K => return Ok(i.k),
            c => return Err(format!("an instruction 0x{c:x} this evaluator does not know")),
        }
    }
    Err("the program does not end".into())
}

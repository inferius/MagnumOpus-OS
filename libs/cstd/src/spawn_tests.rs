//! Testy `posix_spawn` a `waitpid` na hostiteli: akce souboru (`plan`),
//! hledani programu, stav potomka a tabulka potomku nad nahradnimi
//! potomky `sys::host` (`exit:<kod>`, `sleep:<ms>:<kod>`).

use crate::errno::{EBADF, ECHILD, EINVAL, ENOENT, ENOTSUP, EPERM};
use crate::signal::{SIGABRT, SIGKILL, SIGSEGV, SIGTERM, SIGUSR1};
use crate::spawn::wait::*;
use crate::spawn::*;
use crate::sys::ChildIo::{self, Closed, Null, Parent};

fn no_dir(_: crate::c_int) -> Result<Vec<u8>, crate::c_int> {
    Err(EBADF)
}

#[test]
fn plan_of_file_actions() {
    let p = plan(&[], b"/home".to_vec(), no_dir).unwrap();
    assert_eq!(p.io, [Parent(0), Parent(1), Parent(2)]);
    assert_eq!(p.cwd, b"/home");
    // Roura rodice na stdout, zavreni obou koncu (vyssi cisla = nic).
    let a = [Action::Dup2(6, 1), Action::Close(5), Action::Close(6)];
    assert_eq!(plan(&a, b"/".to_vec(), no_dir).unwrap().io, [Parent(0), Parent(6), Parent(2)]);
    // 2>&1 po presmerovani stdout.
    let a = [Action::Dup2(7, 1), Action::Dup2(1, 2)];
    assert_eq!(plan(&a, b"/".to_vec(), no_dir).unwrap().io, [Parent(0), Parent(7), Parent(7)]);
    let a = [Action::Close(0), Action::Open(1, b"/dev/null".to_vec())];
    assert_eq!(plan(&a, b"/".to_vec(), no_dir).unwrap().io, [Closed, Null, Parent(2)]);
    assert_eq!(plan(&[Action::Close(0), Action::Dup2(0, 1)], b"/".to_vec(), no_dir), Err(EBADF));
    assert_eq!(plan(&[Action::Dup2(1, 5)], b"/".to_vec(), no_dir), Err(ENOTSUP));
    assert_eq!(plan(&[Action::Open(1, b"/tmp/out.txt".to_vec())], b"/".to_vec(), no_dir), Err(ENOTSUP));
    assert_eq!(plan(&[Action::Open(4, b"/dev/null".to_vec())], b"/".to_vec(), no_dir), Err(ENOTSUP));
    let a = [Action::CloseFrom(1)];
    assert_eq!(plan(&a, b"/".to_vec(), no_dir).unwrap().io, [Parent(0), Closed, Closed]);
    assert_eq!(plan(&[Action::CloseFrom(3)], b"/".to_vec(), no_dir).unwrap().io, [Parent(0), Parent(1), Parent(2)]);
    // Adresar: relativni vuci predchozimu, fd pres `dir_of`.
    let a = [Action::Chdir(b"src".to_vec()), Action::Chdir(b"lib".to_vec())];
    assert_eq!(plan(&a, b"/home/u".to_vec(), no_dir).unwrap().cwd, b"/home/u/src/lib");
    let dir = |fd| if fd == 9 { Ok(b"/data".to_vec()) } else { Err(EBADF) };
    let a = [Action::Fchdir(9), Action::Chdir(b"x".to_vec())];
    assert_eq!(plan(&a, b"/".to_vec(), dir).unwrap().cwd, b"/data/x");
    assert_eq!(plan(&[Action::Fchdir(3)], b"/".to_vec(), dir), Err(EBADF));
}

#[test]
fn program_lookup() {
    let have = |p: &[u8]| p == b"/home/u/tool" || p == b"/opt/bin/cc";
    assert_eq!(resolve_program(b"/bin/x", b"/home/u", None, have), b"/bin/x");
    assert_eq!(resolve_program(b"sub/x", b"/home/u", None, have), b"/home/u/sub/x");
    assert_eq!(resolve_program(b"tool", b"/home/u", None, have), b"/home/u/tool");
    // Holy nazev bez souboru = program systemu pro launchd.
    assert_eq!(resolve_program(b"ls", b"/home/u", None, have), b"ls");
    assert_eq!(resolve_program(b"cc", b"/home/u", Some(b"/usr/bin:/opt/bin"), have), b"/opt/bin/cc");
    assert_eq!(resolve_program(b"tool", b"/home/u", Some(b"/usr/bin::/opt/bin"), have), b"/home/u/tool");
    assert_eq!(resolve_program(b"ls", b"/home/u", Some(b"/usr/bin"), have), b"ls");
}

#[test]
fn wait_status_encoding() {
    assert_eq!(status_of(0, None), 0);
    assert_eq!(status_of(3, None), 3 << 8);
    assert_eq!(status_of(256 + 7, None), 7 << 8, "jen spodnich 8 bitu");
    assert_eq!(status_of(-1, None), 0xff << 8);
    assert_eq!(status_of(-1_000_000, None), SIGKILL);
    assert_eq!(status_of(-1_000_000, Some(SIGTERM)), SIGTERM);
    assert_eq!(status_of(-1_000_001, None), SIGABRT);
    assert_eq!(status_of(-1_000_002, None), 0);
    // Page fault (vektor 14).
    assert_eq!(status_of(-1_000_114, None), SIGSEGV);
}

/// Spusti nahradniho potomka primo pres sys (bez posix_spawn).
fn child(prog: &str) -> crate::c_int {
    let io: [ChildIo; 3] = [Parent(0), Parent(1), Parent(2)];
    let req = crate::sys::SpawnReq { program: prog.into(), args: vec![], env: vec![], cwd: b"/".to_vec(), io };
    register(crate::sys::spawn_child(req).unwrap())
}

#[test]
fn waitpid_on_children() {
    let a = child("exit:3");
    let b = child("sleep:40:0");
    assert!(a > 1 && b > a);
    let mut st = 0;
    // SAFETY: platne ukazatele.
    unsafe {
        assert_eq!(waitpid(b, &mut st, WNOHANG), 0, "jeste bezi");
        assert_eq!(waitpid(a, &mut st, 0), a);
        assert_eq!(st, 3 << 8);
        assert_eq!(waitpid(a, &mut st, 0), -1, "uz sklizeny");
        assert_eq!(crate::errno::get(), ECHILD);
        assert_eq!(waitpid(b, &mut st, 0), b);
        assert_eq!(st, 0);
        assert_eq!(waitpid(1, &mut st, 9999), -1);
        assert_eq!(crate::errno::get(), EINVAL);
    }
}

#[test]
fn kill_child_reports_signal() {
    let c = child("sleep:60000:0");
    assert_eq!(crate::signal::kill(c, 0), 0);
    assert_eq!(crate::signal::kill(c, SIGUSR1), -1);
    assert_eq!(crate::errno::get(), ENOTSUP);
    assert_eq!(crate::signal::kill(c, SIGTERM), 0);
    let mut st = 0;
    // SAFETY: platny ukazatel.
    assert_eq!(unsafe { waitpid(c, &mut st, 0) }, c);
    assert_eq!(st, SIGTERM);
    assert_eq!(crate::signal::kill(c, SIGTERM), -1, "potomek uz neni");
}

#[test]
fn spawn_api_and_errors() {
    let mut fa = core::mem::MaybeUninit::<posix_spawn_file_actions_t>::uninit();
    let mut at = core::mem::MaybeUninit::<posix_spawnattr_t>::uninit();
    assert_eq!(size_of::<posix_spawn_file_actions_t>(), 80);
    assert_eq!(size_of::<posix_spawnattr_t>(), 336);
    // SAFETY: struktury inicializuji funkce init.
    unsafe {
        let (fa, at) = (fa.as_mut_ptr(), at.as_mut_ptr());
        assert_eq!(posix_spawn_file_actions_init(fa), 0);
        assert_eq!(posix_spawn_file_actions_addclose(fa, -1), EBADF);
        assert_eq!(posix_spawn_file_actions_adddup2(fa, 1, 2), 0);
        assert_eq!(posix_spawnattr_init(at), 0);
        assert_eq!(posix_spawnattr_setflags(at, 0x100), EINVAL);
        assert_eq!(posix_spawnattr_setflags(at, POSIX_SPAWN_SETSID | POSIX_SPAWN_SETSIGMASK), 0);
        let mut f = 0;
        assert_eq!(posix_spawnattr_getflags(at, &mut f), 0);
        assert_eq!(f, POSIX_SPAWN_SETSID | POSIX_SPAWN_SETSIGMASK);
        let argv = [c"exit:5".as_ptr(), core::ptr::null()];
        let mut pid = 0;
        assert_eq!(posix_spawn(&mut pid, c"exit:5".as_ptr().cast(), fa, at, argv.as_ptr().cast(), core::ptr::null()), 0);
        let mut st = 0;
        assert_eq!(waitpid(pid, &mut st, 0), pid);
        assert_eq!(st, 5 << 8);
        assert_eq!(posix_spawn(&mut pid, c"neni".as_ptr().cast(), fa, at, argv.as_ptr().cast(), core::ptr::null()), ENOENT);
        // Skupina procesu jina nez nova = ENOTSUP, realny cas = EPERM.
        assert_eq!(posix_spawnattr_setflags(at, POSIX_SPAWN_SETPGROUP), 0);
        assert_eq!(posix_spawnattr_setpgroup(at, 77), 0);
        assert_eq!(posix_spawn(&mut pid, c"exit:0".as_ptr().cast(), fa, at, argv.as_ptr().cast(), core::ptr::null()), ENOTSUP);
        assert_eq!(posix_spawnattr_setflags(at, POSIX_SPAWN_SETSCHEDULER), 0);
        assert_eq!(posix_spawnattr_setschedpolicy(at, 1), 0);
        assert_eq!(posix_spawn(&mut pid, c"exit:0".as_ptr().cast(), fa, at, argv.as_ptr().cast(), core::ptr::null()), EPERM);
        assert_eq!(posix_spawnattr_setschedpolicy(at, 9), EINVAL);
        // Neexistujici adresar potomka.
        assert_eq!(posix_spawnattr_setflags(at, 0), 0);
        assert_eq!(posix_spawn_file_actions_addchdir_np(fa, c"/neni/takovy/adresar".as_ptr().cast()), 0);
        assert_eq!(posix_spawn(&mut pid, c"exit:0".as_ptr().cast(), fa, at, argv.as_ptr().cast(), core::ptr::null()), ENOENT);
        assert_eq!(posix_spawn_file_actions_destroy(fa), 0);
        assert_eq!(posix_spawnattr_destroy(at), 0);
    }
}

#[test]
fn wait_header_matches() {
    let h = include_str!("../include/sys/wait.h");
    for (n, v) in [("WNOHANG", WNOHANG), ("WUNTRACED", WUNTRACED), ("WEXITED", WEXITED), ("WCONTINUED", WCONTINUED)] {
        assert!(h.contains(&format!("#define {n} {v}\n")), "{n}");
    }
    let s = include_str!("../include/spawn.h");
    for (n, v) in [("SETPGROUP", POSIX_SPAWN_SETPGROUP), ("SETSIGMASK", POSIX_SPAWN_SETSIGMASK), ("SETSID", POSIX_SPAWN_SETSID)] {
        assert!(s.contains(&format!("#define POSIX_SPAWN_{n} 0x{v:02x}\n")), "{n}");
    }
}

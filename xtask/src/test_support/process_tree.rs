use super::{Result, spawn, wait_file, write_json};
use serde_json::json;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process, thread,
    time::{Duration, Instant},
};

pub fn agent(args: &[String]) -> Result {
    let role = args.first().ok_or("Missing process tree role")?;
    if role == "leaf" {
        write_json("tree-leaf-ready.json", &json!({"pid":process::id()}))?;
        thread::sleep(Duration::from_secs(60));
        return Ok(());
    }
    let parent = args.get(1).ok_or("Missing adapter PID")?.parse::<u32>()?;
    let detach = args.iter().any(|s| s == "--detach");
    let mut child = spawn(&["agent-tree".into(), "leaf".into()], detach)?;
    wait_file(Path::new("tree-leaf-ready.json"), &mut child, 10)?;
    let mut pids = json!({"adapter":parent,"branch":process::id(),"leaf":child.id()});
    if let Some(index) = args.iter().position(|s| s == "--guardian") {
        pids["guardian"] = json!(
            args.get(index + 1)
                .ok_or("Missing guardian")?
                .parse::<u32>()?
        );
    }
    write_json("tree-ready.json", &pids)?;
    thread::sleep(Duration::from_secs(60));
    if child.try_wait()?.is_none() {
        child.kill()?;
    }
    child.wait()?;
    Ok(())
}

pub fn browser(args: &[String]) -> Result {
    let directory = args.first().ok_or("Missing browser fixture directory")?;
    std::env::set_current_dir(directory)?;
    #[cfg(windows)]
    if args.get(1).is_none_or(|s| s.starts_with("--")) {
        spawn(
            &[
                "agent-tree".into(),
                "branch".into(),
                process::id().to_string(),
            ],
            true,
        )?;
        thread::sleep(Duration::from_secs(60));
        return Ok(());
    }
    let mode = args.get(1).ok_or("Missing browser fixture mode")?;
    let role = args.get(2).ok_or("Missing browser fixture role")?;
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
    }
    if role == "supervisor" {
        let mut header = [0; 8];
        std::io::stdin().read_exact(&mut header)?;
        assert_eq!(&header, b"LPBO\x02\0\r\n");
        let data: &[u8] = match mode.as_str() {
            "partial" => br#"{"pid":"#,
            "oversized" => &[b'x'; 4097],
            "invalid" => b"not-json\n",
            "eof" => b"",
            _ => return Err("Unknown malformed browser mode".into()),
        };
        std::io::stdout().write_all(data)?;
        std::io::stdout().flush()?;
        write_json("status-ready.json", &json!({}))?;
        if mode != "eof" {
            #[cfg(unix)]
            {
                let mut poll = libc::pollfd {
                    fd: 0,
                    events: libc::POLLIN,
                    revents: 0,
                };
                unsafe {
                    libc::poll(&mut poll, 1, 3000);
                }
            }
            #[cfg(not(unix))]
            thread::sleep(Duration::from_secs(3));
        }
        return Ok(());
    }
    if role == "root" {
        let profile = args
            .iter()
            .find_map(|s| s.strip_prefix("--user-data-dir="))
            .ok_or("Missing browser profile")?;
        fs::write(Path::new(profile).join("fixture-cache"), b"owned-profile\n")?;
        write_json("browser-profile.json", &json!({"path":profile}))?;
    }
    if role == "leaf" {
        write_json("browser-leaf.json", &json!({"pid":process::id()}))?;
        #[cfg(unix)]
        if mode == "adopt" {
            let guardian = args
                .get(4)
                .ok_or("Missing adopting guardian")?
                .parse::<u32>()?;
            let end = Instant::now() + Duration::from_secs(5);
            while unsafe { libc::getppid() } as u32 != guardian {
                assert!(Instant::now() < end, "Browser orphan was not adopted");
                thread::sleep(Duration::from_millis(5));
            }
            write_json(
                "adopted.json",
                &json!({"pid":process::id(),"parent":unsafe{libc::getppid()}}),
            )?;
            process::exit(0);
        }
    } else {
        let root = if role == "root" {
            process::id()
        } else {
            args.get(3).ok_or("Missing root PID")?.parse::<u32>()?
        };
        #[cfg(unix)]
        let guardian = if role == "root" {
            (unsafe { libc::getppid() }) as u32
        } else {
            args.get(4).ok_or("Missing guardian")?.parse::<u32>()?
        };
        #[cfg(not(unix))]
        let guardian = 0u32;
        let mut child = spawn(
            &[
                "browser-tree".into(),
                directory.clone(),
                mode.clone(),
                if role == "root" {
                    "branch".into()
                } else {
                    "leaf".into()
                },
                root.to_string(),
                guardian.to_string(),
            ],
            mode == "detach",
        )?;
        if role == "branch" {
            wait_file(Path::new("browser-leaf.json"), &mut child, 5)?;
            write_json(
                "browser-tree.json",
                &json!({"root":root,"branch":process::id(),"leaf":child.id(),"guardian":guardian}),
            )?;
            if mode == "adopt" {
                process::exit(0);
            }
        } else if mode == "exit" {
            wait_file(Path::new("browser-tree.json"), &mut child, 5)?;
            process::exit(91);
        }
    }
    thread::sleep(Duration::from_secs(60));
    Ok(())
}

#[cfg(unix)]
pub fn input_worker(args: &[String]) -> Result {
    let path = args.last().ok_or("Missing input fixture path")?;
    let child = unsafe { libc::fork() };
    if child < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if child == 0 {
        thread::sleep(Duration::from_secs(60));
        unsafe { libc::_exit(0) }
    }
    write_json(path, &json!([process::id(), child]))?;
    if path.ends_with("exited.json") {
        return Ok(());
    }
    thread::sleep(Duration::from_secs(60));
    Ok(())
}

#[cfg(not(unix))]
pub fn input_worker(_args: &[String]) -> Result {
    Err("This fixture requires POSIX process inheritance".into())
}

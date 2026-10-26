use super::*;
use std::io::Read;
use std::net::TcpListener;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn test_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "wb2api-browser-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    root
}

#[test]
fn account_and_realm_environments_are_separate_and_stable() {
    let root = test_root();
    let a = Profile::prepare(&root, "cn", None).unwrap();
    let b = Profile::prepare(&root, "cn", None).unwrap();
    assert_ne!(a.id, b.id);
    assert_ne!(a.audio_seed, b.audio_seed);
    fs::write(
        a.directory(&root).unwrap().join("profile/cookie-fixture"),
        "account-a-only",
    )
    .unwrap();
    assert!(
        !b.directory(&root)
            .unwrap()
            .join("profile/cookie-fixture")
            .exists()
    );
    a.bind(&root, "account-a").unwrap();
    let reopened = Profile::prepare(&root, "cn", Some("account-a")).unwrap();
    assert_eq!(reopened.id, a.id);
    assert_eq!(reopened.config(), a.config());
    assert!(reopened.check_account("account-b").is_err());
    let global = Profile::prepare(&root, "global", Some("account-a")).unwrap();
    assert_ne!(global.id, a.id);
    assert!(Profile::prepare(&root, "../../escape", None).is_err());
    let mut invalid = a;
    invalid.id = "../escape".into();
    assert!(invalid.directory(&root).is_err());
    fs::remove_dir_all(root).unwrap();
}

// Opt-in native test: launches the actual bundled engine headlessly against a
// loopback-only fixture, without logging in or reading the user's browser data.
#[test]
#[ignore = "launches the bundled native browser; run explicitly"]
fn native_browser_isolates_cookies_storage_and_fingerprints() {
    let root = test_root();
    let a = Profile::prepare(&root, "cn", None).unwrap();
    let b = Profile::prepare(&root, "cn", None).unwrap();
    let one = report(&root, &a);
    let two = report(&root, &b);
    let again = report(&root, &a);
    assert_eq!(one["before"], Value::Null);
    assert_eq!(two["before"], Value::Null);
    assert_eq!(one["cookieBefore"], "");
    assert_eq!(two["cookieBefore"], "");
    assert_eq!(again["before"], one["marker"]);
    assert!(
        again["cookieBefore"]
            .as_str()
            .unwrap()
            .contains(one["marker"].as_str().unwrap())
    );
    assert_eq!(one["cores"].as_u64(), Some(a.cpu_cores as u64));
    assert_eq!(two["cores"].as_u64(), Some(b.cpu_cores as u64));
    assert_ne!(
        one["audio"], two["audio"],
        "account fingerprint seeds must differ natively"
    );
    assert_eq!(
        one["audio"], again["audio"],
        "returning account fingerprint must remain stable"
    );
    println!(
        "native Camoufox {VERSION}: complete long launch URL including query + fragment, independent cookies + localStorage, distinct and stable audio fingerprints, native CPU configuration verified"
    );
    // The profile is written only under this test-owned absolute temp directory.
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn parent_lock_is_in_use_only_while_someone_holds_it() {
    let root = test_root();
    let profile = Profile::prepare(&root, "cn", None).unwrap();
    let dir = profile.directory(&root).unwrap().join("profile");

    assert!(!profile_in_use(&dir), "无锁文件时不得判为占用");

    // 浏览器被强杀后锁文件会残留，存在本身不等于有人在用。
    fs::write(dir.join("parent.lock"), "").unwrap();
    assert!(!profile_in_use(&dir), "陈旧锁残留时不得判为占用");

    // Camoufox 以独占方式持有该文件，复现同样的共享模式即可验证判活方向。
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let held = OpenOptions::new()
            .write(true)
            .share_mode(0)
            .open(dir.join("parent.lock"))
            .unwrap();
        assert!(profile_in_use(&dir), "锁被持有时必须判为占用");
        drop(held);
    }
    assert!(!profile_in_use(&dir), "释放后应回到未占用");
    fs::remove_dir_all(root).unwrap();
}

fn report(root: &Path, profile: &Profile) -> Value {
    // Reuse one origin across all profiles and launches so isolation is observable.
    // Origin is fixed per test process, rather than allocating a different origin
    // for each browser (which would hide storage leaks).
    static LISTENER: OnceLock<TcpListener> = OnceLock::new();
    let listener = LISTENER
        .get_or_init(|| TcpListener::bind("127.0.0.1:0").unwrap())
        .try_clone()
        .unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://127.0.0.1:{}/authorize?platform=CLI&state=synthetic-only&redirect_uri=https%3A%2F%2Fexample.invalid%2Freturn%3Fa%3D1%26b%3D2&encoded=%2526%2523%253F%3D%2B&long={}#route?token=fixture%26kept&last=yes",
        listener.local_addr().unwrap().port(),
        "a".repeat(2048)
    );
    let mut command = profile.command(root, &url).unwrap();
    OpenOptions::new()
        .append(true)
        .open(profile.directory(root).unwrap().join("profile/user.js"))
        .unwrap()
        .write_all(b"user_pref(\"dom.allow_scripts_to_close_windows\", true);\n")
        .unwrap();
    command.arg("-headless");
    let mut child = command.spawn().unwrap();
    let started = Instant::now();
    let mut result: Option<Value> = None;
    while started.elapsed() < Duration::from_secs(35) {
        match listener.accept() {
            Ok((mut socket, _)) => {
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") && request.len() < 16384 {
                    match socket.read(&mut byte) {
                        Ok(1) => request.push(byte[0]),
                        _ => break,
                    }
                }
                let header = String::from_utf8_lossy(&request);
                if header.starts_with("POST /report ") {
                    let length = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|s| s.trim().parse::<usize>().ok())
                        })
                        .unwrap();
                    let mut payload = vec![0; length];
                    socket.read_exact(&mut payload).unwrap();
                    result = Some(serde_json::from_slice(&payload).unwrap());
                    let _ = socket.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK",
                    );
                    break;
                }
                let html = r#"<!doctype html><meta charset=utf-8><title>WorkBuddy Login Isolation Test</title><script>
                (async()=>{try{
                  const before=localStorage.getItem('marker'), cookieBefore=document.cookie;
                  const marker=before||crypto.randomUUID(); localStorage.setItem('marker',marker);
                  document.cookie='marker='+marker+'; Max-Age=3600; SameSite=Lax; Path=/';
                  const ctx=new OfflineAudioContext(1,10000,44100), osc=ctx.createOscillator(), comp=ctx.createDynamicsCompressor();
                  osc.type='triangle';osc.frequency.value=1000;osc.connect(comp);comp.connect(ctx.destination);osc.start();
                  const audio=await ctx.startRendering();
                  const digest=await crypto.subtle.digest('SHA-256',audio.getChannelData(0).buffer);
                  const hash=Array.from(new Uint8Array(digest),x=>x.toString(16).padStart(2,'0')).join('');
                  await fetch('/report',{method:'POST',body:JSON.stringify({before,cookieBefore,marker,audio:hash,cores:navigator.hardwareConcurrency,href:location.href})});window.close();
                }catch(e){await fetch('/report',{method:'POST',body:JSON.stringify({error:String(e)})})}})();
                </script>"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                    html.len()
                );
                let _ = socket.write_all(response.as_bytes());
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(error) => panic!("fixture listener: {error}"),
        }
    }
    // The test page closes its window so Firefox flushes its persistent stores.
    let deadline = Instant::now() + Duration::from_secs(6);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    if child.try_wait().unwrap().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    std::thread::sleep(Duration::from_millis(600));
    let result = result.expect("bundled browser did not report to the local fixture");
    assert!(
        result.get("error").is_none(),
        "browser fixture error: {result}"
    );
    assert_eq!(
        result["href"].as_str(),
        Some(url.as_str()),
        "native browser lost URL query parameters or fragment"
    );
    result
}

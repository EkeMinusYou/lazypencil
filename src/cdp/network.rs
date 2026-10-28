//! An owned, non-forwarding proxy for isolated page contexts. No requests are retained.

#[cfg(unix)]
use super::failed;
use super::{Browser, Budget};
use crate::{Result, config::runtime::PageNetworkPolicy};
use std::net::SocketAddr;

#[cfg(unix)]
const REJECTION: &[u8] = b"HTTP/1.1 502 Offline\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

pub(super) struct DenyProxy {
    pub address: SocketAddr,
    #[cfg(unix)]
    wake: std::os::unix::net::UnixStream,
    #[cfg(unix)]
    task: Option<std::thread::JoinHandle<()>>,
}

pub(crate) fn check_network_navigation(
    policy: crate::config::runtime::PageNetworkPolicy,
    url: &str,
) -> Result<()> {
    if policy == crate::config::runtime::PageNetworkPolicy::Offline
        && matches!(
            url::Url::parse(url)
                .map_err(|_| crate::Error::new("PAGE_LOAD_FAILED", "Invalid browser URL"))?
                .scheme(),
            "http" | "https"
        )
    {
        return Err(crate::Error::new(
            "PAGE_LOAD_FAILED",
            "Offline web policy forbids a remote main document",
        ));
    }
    Ok(())
}

pub(crate) fn check_network_frame(
    policy: PageNetworkPolicy,
    frame: &serde_json::Value,
) -> Result<()> {
    for key in ["url", "unreachableUrl"] {
        if let Some(url) = frame[key].as_str().filter(|url| !url.is_empty()) {
            check_network_navigation(policy, url)?;
        }
    }
    Ok(())
}

impl Browser {
    pub(super) fn check_page_network_location(
        &mut self,
        session: &str,
        budget: &Budget<'_>,
    ) -> Result<()> {
        if self.deny_proxy.is_some() {
            let frame = self.command(
                Some(session),
                "Page.getFrameTree",
                serde_json::json!({}),
                budget,
            )?;
            check_network_frame(PageNetworkPolicy::Offline, &frame["frameTree"]["frame"])?;
        }
        Ok(())
    }
}

#[cfg(unix)]
impl DenyProxy {
    pub(super) fn new() -> Result<Self> {
        use std::{
            io::Write, net::TcpListener, os::fd::AsRawFd, os::unix::net::UnixStream, time::Duration,
        };
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| failed(format!("Cannot bind the offline web proxy: {e}")))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| failed(e.to_string()))?;
        let address = listener.local_addr().map_err(|e| failed(e.to_string()))?;
        let (wake, reader) = UnixStream::pair().map_err(|e| failed(e.to_string()))?;
        wake.set_nonblocking(true)
            .map_err(|e| failed(e.to_string()))?;
        let task = std::thread::Builder::new()
            .name("lazypencil-offline-web".into())
            .spawn(move || {
                let mut descriptors = [
                    libc::pollfd {
                        fd: listener.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    },
                    libc::pollfd {
                        fd: reader.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    },
                ];
                loop {
                    // Wake through a local socket pair, independent of network/firewall state.
                    let result = unsafe {
                        libc::poll(
                            descriptors.as_mut_ptr(),
                            descriptors.len() as libc::nfds_t,
                            -1,
                        )
                    };
                    if result < 0 {
                        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                        {
                            continue;
                        }
                        break;
                    }
                    if descriptors[1].revents != 0 {
                        break;
                    }
                    if descriptors[0].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)
                        != 0
                    {
                        break;
                    }
                    if descriptors[0].revents & libc::POLLIN == 0 {
                        continue;
                    }
                    let mut stream = match listener.accept() {
                        Ok((stream, _)) => stream,
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                            ) =>
                        {
                            continue;
                        }
                        Err(_) => break,
                    };
                    if stream
                        .set_write_timeout(Some(Duration::from_millis(200)))
                        .is_err()
                    {
                        continue;
                    }
                    // Reject HTTP, CONNECT and WebSocket upgrades before reading anything.
                    let _ = stream.write_all(REJECTION);
                }
            })
            .map_err(|e| failed(format!("Cannot start the offline web proxy: {e}")))?;
        Ok(Self {
            address,
            wake,
            task: Some(task),
        })
    }
}

#[cfg(not(unix))]
impl DenyProxy {
    pub(super) fn new() -> Result<Self> {
        Err(crate::Error::new(
            "UNSUPPORTED_FEATURE",
            "Offline web acquisition is available on macOS and Linux",
        ))
    }
}

#[cfg(unix)]
impl Drop for DenyProxy {
    fn drop(&mut self) {
        use std::io::Write;
        let _ = self.wake.write_all(&[1]);
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_main_document_policy_checks_parsed_schemes_without_leaking_urls() {
        use crate::config::runtime::PageNetworkPolicy::{Offline, Online};
        for url in [
            "http://example.test/",
            " \nHTTPS://example.test/?token=private-value",
            "h\tttp://example.test/",
        ] {
            let error = check_network_navigation(Offline, url).unwrap_err();
            assert_eq!(error.code, "PAGE_LOAD_FAILED");
            assert!(!error.message.contains("private-value"));
            check_network_navigation(Online, url).unwrap();
        }
        for url in [
            "about:blank",
            "data:text/html,local",
            "file:///original/index.html",
            "blob:https://example.test/id",
        ] {
            check_network_navigation(Offline, url).unwrap();
        }
        let error_page = serde_json::json!({
            "url":"chrome-error://chromewebdata/",
            "unreachableUrl":"https://example.test/?token=private-value"
        });
        let error = check_network_frame(Offline, &error_page).unwrap_err();
        assert_eq!(error.code, "PAGE_LOAD_FAILED");
        assert!(!error.message.contains("private-value"));
        check_network_frame(Online, &error_page).unwrap();
        check_network_frame(
            Offline,
            &serde_json::json!({"url":"about:blank","unreachableUrl":""}),
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn offline_proxy_rejects_without_headers_and_reaps_while_a_client_is_idle() {
        use std::{
            io::Read,
            net::TcpStream,
            time::{Duration, Instant},
        };
        let proxy = DenyProxy::new().unwrap();
        let mut client =
            TcpStream::connect_timeout(&proxy.address, Duration::from_secs(2)).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
        assert_eq!(
            response,
            b"HTTP/1.1 502 Offline\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let _idle = TcpStream::connect_timeout(&proxy.address, Duration::from_secs(2)).unwrap();
        let started = Instant::now();
        drop(proxy);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}

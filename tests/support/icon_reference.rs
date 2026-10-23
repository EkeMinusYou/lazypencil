//! Original-source browser references, served only on a temporary loopback port.

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub struct Server {
    pub address: SocketAddr,
    stop: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<()>>,
}
impl Server {
    pub fn new(sources: &Path, phosphor: &Path) -> Self {
        let mut fonts = BTreeMap::new();
        let mut codes = BTreeMap::new();
        for style in ["Outlined", "Rounded", "Sharp"] {
            let name = format!("MaterialSymbols{style}[FILL,GRAD,opsz,wght]");
            let bytes = std::fs::read(sources.join(format!("{name}.ttf"))).unwrap();
            let descriptor = lazypencil::icons::catalogue()
                .iter()
                .find(|d| d.id == format!("Material Symbols {style}"))
                .unwrap();
            assert_eq!(lazypencil::document::hash(&bytes), descriptor.source_sha256);
            let index = std::fs::read(sources.join(format!("{name}.codepoints"))).unwrap();
            assert_eq!(
                Some(lazypencil::document::hash(&index)),
                descriptor.index_sha256
            );
            let text = std::str::from_utf8(&index).unwrap();
            let code = text
                .lines()
                .find_map(|line| line.strip_prefix("settings "))
                .unwrap()
                .to_owned();
            fonts.insert(style.to_string(), bytes);
            codes.insert(style.to_string(), code);
        }
        let archive = std::fs::read(phosphor).unwrap();
        assert_eq!(
            lazypencil::document::hash(&archive),
            lazypencil::icons::catalogue()
                .iter()
                .find(|d| d.id == "phosphor")
                .unwrap()
                .source_sha256
        );
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(&archive[..]));
        let mut svgs = BTreeMap::new();
        for item in archive.entries().unwrap() {
            let mut item = item.unwrap();
            let path = item.path().unwrap().into_owned();
            if let Some(name) = path
                .to_str()
                .unwrap()
                .strip_prefix("package/assets/regular/")
                .and_then(|n| n.strip_suffix(".svg"))
            {
                let mut svg = String::new();
                item.read_to_string(&mut svg).unwrap();
                svgs.insert(name.to_string(), svg);
            }
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let task = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                // BSD sockets can inherit the listener's nonblocking flag.
                stream.set_nonblocking(false).unwrap();
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let mut header = Vec::new();
                let mut block = [0; 1024];
                while header.len() < 8192 && !header.windows(4).any(|v| v == b"\r\n\r\n") {
                    match stream.read(&mut block) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => header.extend_from_slice(&block[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&header);
                let route = text.split_ascii_whitespace().nth(1).unwrap_or("");
                eprintln!("Icon reference HTTP {route}");
                let mut body = Vec::new();
                let mut kind = "text/html";
                let mut status = "200 OK";
                if let Some(style) = route.strip_prefix("/font/") {
                    if let Some(font) = fonts.get(style) {
                        body = font.clone();
                        kind = "font/ttf";
                    } else {
                        status = "404 Not Found";
                    }
                } else if let Some(name) = route.strip_prefix("/phosphor/") {
                    if let Some(svg) = svgs.get(name) {
                        body=format!("<style>html,body{{margin:0;background:white;color:#123456}}svg{{display:block;width:96px;height:48px}}</style>{svg}").into_bytes();
                    } else {
                        status = "404 Not Found";
                    }
                } else if let Some((style, weight)) = route.trim_start_matches('/').split_once('/')
                {
                    if codes.contains_key(style)
                        && weight
                            .parse::<f32>()
                            .is_ok_and(|w| (100.0..=700.0).contains(&w))
                    {
                        body=format!("<style>html,body{{margin:0;background:white}}@font-face{{font-family:P;src:url('/font/{style}') format('truetype');font-weight:100 700}}svg{{display:block}}text{{font-family:P;font-size:96px;font-weight:{weight};text-rendering:geometricPrecision;font-optical-sizing:none;font-synthesis:none;font-variation-settings:'wght' {weight},'opsz' 24,'FILL' 0,'GRAD' 0}}</style><svg xmlns='http://www.w3.org/2000/svg' width='96' height='96'><text x='0' y='96' fill='#123456'>&#x{};</text></svg>",codes[style]).into_bytes();
                    } else {
                        status = "404 Not Found";
                    }
                } else {
                    status = "404 Not Found";
                }
                if kind == "text/html" && status == "200 OK" && !route.starts_with("/phosphor/") {
                    body.extend_from_slice(b"<script>window.addEventListener('load',()=>document.fonts.ready.then(()=>fetch('/probe?'+encodeURIComponent([...document.fonts].map(f=>f.family+':'+f.status).join(',')))));</script>");
                }
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
                    body.len()
                );
                if stream.write_all(header.as_bytes()).is_ok()
                    && let Err(error) = stream.write_all(&body)
                {
                    eprintln!("Icon reference transfer: {error}");
                }
            }
        });
        Self {
            address,
            stop,
            task: Some(task),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

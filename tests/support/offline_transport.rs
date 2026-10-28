//! Exercise transport routing through the application for fresh and retained pages.

use super::*;

#[test]
#[ignore = "Requires sandboxed headless Chromium and owned UDP/TCP sinks"]
fn offline_page_process_blocks_stun_turn_and_webtransport_and_online_resumes() {
    let server = transport_reference::Server::new();
    let mut rows = Vec::new();
    for retained in [false, true] {
        let f = LocalFixture::new("2.21");
        let html = format!(
            r#"<!doctype html><html><head><link rel='icon' href='data:,'><style>html{{height:100%;background:red}}body{{margin:0}}</style></head><body><script>
        (async()=>{{
          let peer,transport,worker,workerURL,timer,ok=false;
          try{{
            peer=new RTCPeerConnection({{iceServers:[{{urls:{stun}}},{{urls:{turn},username:'owned',credential:'owned'}}]}});
            peer.createDataChannel('owned');await peer.setLocalDescription(await peer.createOffer());
            transport=new WebTransport({quic});transport.closed.catch(()=>{{}});transport.ready.catch(()=>{{}});
            const source=`onmessage=async event=>{{let transport;try{{transport=new WebTransport(event.data);transport.closed.catch(()=>{{}});transport.ready.catch(()=>{{}});await new Promise(r=>setTimeout(r,1500));postMessage('observed')}}catch(error){{postMessage('error:'+error.name)}}finally{{if(transport)transport.close()}}}}`;
            workerURL=URL.createObjectURL(new Blob([source],{{type:'text/javascript'}}));
            worker=new Worker(workerURL);
            const observed=new Promise(r=>{{worker.onmessage=e=>r(e.data);worker.onerror=e=>r('error:'+e.message);timer=setTimeout(()=>r('timeout'),4000)}});
            worker.postMessage({worker_quic});
            const results=await Promise.all([new Promise(r=>setTimeout(r,1500)),observed]);document.documentElement.dataset.transportResult=results[1];ok=results[1]==='observed';
          }}catch(error){{document.documentElement.dataset.transportResult='error:'+error.name+':'+error.message.slice(0,160)}}finally{{
            clearTimeout(timer);if(peer)peer.close();if(transport)transport.close();if(worker)worker.terminate();if(workerURL)URL.revokeObjectURL(workerURL);
            document.documentElement.style.background=ok?'#00ff00':'#ff0000';document.documentElement.dataset.transport='yes';
          }}
        }})();
        </script></body></html>"#,
            stun = json!(format!("stun:{}", server.stun)),
            turn = json!(format!("turn:{}?transport=tcp", server.tcp.address)),
            quic = json!(format!("https://{}/owned", server.quic)),
            worker_quic = json!(format!("https://{}/owned-worker", server.worker_quic))
        );
        fs::write(f.site().join("index.html"), html).unwrap();
        let policy = |name: &str| {
            fs::write(f.temp.path().join("lazypencil.toml"),format!("[runtime.browser]\nnetwork_policy='{name}'\nready_selector=\"html[data-transport='yes']\"\nselector_state='attached'\ntimeout_ms=15000\n")).unwrap();
        };
        let mut runtime = if retained {
            f.runtime().with_retained_pages(f.store.path())
        } else {
            f.runtime()
        };
        let capture = |runtime: &mut DynamicRuntime, stage: &str, refresh: bool| {
            let result = f.capture_options(
                runtime,
                &RenderOptions {
                    refresh,
                    ..Default::default()
                },
                None,
            );
            if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
                fs::create_dir_all(&directory).unwrap();
                fs::write(std::path::Path::new(&directory).join("codec-offline-transports-observation.json"),serde_json::to_vec_pretty(&json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"retained":retained,"stage":stage,"STUN_TURN_pageQUIC_workerQUIC_counts":server.counts(),"error":result.as_ref().err().map(|error|json!({"code":error.code,"message":error.message}))})).unwrap()).unwrap();
            }
            result.unwrap_or_else(|error| {
                panic!(
                    "{retained}/{stage}: {error:?}; counts={:?}",
                    server.counts()
                )
            })
        };
        let mut keep = |stage: &str, image: &RenderedImage| {
            let raster = Image::from_encoded(Data::new_copy(&image.png))
                .unwrap()
                .make_raster_image(None, None)
                .unwrap();
            let pixels = raster.peek_pixels().unwrap();
            assert_eq!((image.width, image.height), (32, 32));
            for y in 0..32 {
                for x in 0..32 {
                    assert_eq!(
                        pixels.get_color((x, y)),
                        Color::GREEN,
                        "{retained}/{stage}/{x},{y}"
                    );
                }
            }
            let name = "codec-offline-transports.png";
            if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
                let path = std::path::Path::new(&directory).join(name);
                if path.exists() {
                    assert_eq!(fs::read(path).unwrap(), image.png);
                } else {
                    fs::create_dir_all(&directory).unwrap();
                    fs::write(path, &image.png).unwrap();
                }
            }
            assert_eq!(acquisition(image)["page_state"]["retained"], retained);
            let metadata = acquisition(image);
            rows.push(json!({"retained":retained,"stage":stage,"png":name,"png_hash":lazypencil::document::hash(&image.png),"STUN_TURN_pageQUIC_workerQUIC_counts":server.counts(),"network_policy":metadata["network_policy"],"limits_policy":metadata["limits"]["browser"]["network_policy"],"page_state":metadata["page_state"],"acquisition_id":metadata["acquisition_id"],"browser_version":metadata["browser_version"]}));
        };
        policy("online");
        let before = server.counts();
        let online = capture(&mut runtime, "online", false);
        let first = server.counts();
        assert!(
            first.0 > before.0 && first.1 > before.1 && first.2 > before.2 && first.3 > before.3,
            "All positive transport controls must reach their owned sinks: {before:?} -> {first:?}"
        );
        assert_eq!(acquisition(&online)["network_policy"], "online");
        assert_eq!(acquisition(&online)["page_state"]["retained"], retained);
        let online_process = runtime.process_ids();
        assert_eq!(online_process.len(), 1);
        keep("online", &online);
        policy("offline");
        let offline = capture(&mut runtime, "offline", false);
        assert_eq!(
            server.counts(),
            first,
            "Offline STUN/TURN/QUIC must not reach their sinks"
        );
        assert_eq!(acquisition(&offline)["network_policy"], "offline");
        assert_eq!(
            acquisition(&offline)["limits"]["browser"]["network_policy"],
            "offline"
        );
        let offline_process = runtime.process_ids();
        assert_eq!(offline_process.len(), 1);
        assert_ne!(
            offline_process, online_process,
            "Policy changes must replace fresh as well as retained page processes"
        );
        keep("offline", &offline);
        let refreshed = capture(&mut runtime, "offline-refresh", true);
        assert_eq!(server.counts(), first);
        assert_eq!(runtime.process_ids(), offline_process);
        assert_eq!(acquisition(&refreshed)["page_state"]["reused"], retained);
        assert_ne!(
            acquisition(&refreshed)["acquisition_id"],
            acquisition(&offline)["acquisition_id"]
        );
        if retained {
            assert_eq!(
                acquisition(&refreshed)["page_state"]["id"],
                acquisition(&offline)["page_state"]["id"]
            );
        }
        keep("offline-refresh", &refreshed);
        policy("online");
        let resumed = capture(&mut runtime, "online-resumed", false);
        let last = server.counts();
        assert!(
            last.0 > first.0 && last.1 > first.1 && last.2 > first.2 && last.3 > first.3,
            "Online recovery must reach all owned sinks: {first:?} -> {last:?}"
        );
        assert_ne!(runtime.process_ids(), offline_process);
        assert_eq!(acquisition(&resumed)["network_policy"], "online");
        keep("online-resumed", &resumed);
    }
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
        fs::write(std::path::Path::new(&directory).join("codec-offline-transports.json"),serde_json::to_vec_pretty(&json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"cases":rows,"observation_ms":1500,"transport_control":"Actual STUN datagrams, TURN/TCP requests and page/worker QUIC long-header datagrams. Owned sinks intentionally do not complete sessions; positive controls prove routing, not successful peer/media or WebTransport content."})).unwrap()).unwrap();
    }
}

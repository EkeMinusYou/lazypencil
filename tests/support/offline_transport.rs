//! Exercise transport routing through the application for fresh and retained pages.

use super::*;

#[test]
#[ignore = "Requires sandboxed headless Chromium and owned UDP/TCP sinks"]
fn offline_page_process_blocks_stun_turn_and_webtransport_and_online_resumes() {
    let mut rows = Vec::new();
    // Hold both modes' endpoints until all captures finish, including old context teardown.
    let groups = [
        [
            transport_reference::Server::new(),
            transport_reference::Server::new(),
            transport_reference::Server::new(),
        ],
        [
            transport_reference::Server::new(),
            transport_reference::Server::new(),
            transport_reference::Server::new(),
        ],
    ];
    assert_eq!(
        groups
            .iter()
            .flatten()
            .flat_map(|server| [server.stun, server.quic, server.worker_quic])
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        18
    );
    assert_eq!(
        groups
            .iter()
            .flatten()
            .map(|server| server.tcp.address)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        6
    );
    for (mode, retained) in [false, true].into_iter().enumerate() {
        let f = LocalFixture::new("2.21");
        let sinks = &groups[mode];
        let [online_sink, offline_sink, resumed_sink] = sinks;
        let write_page = |server: &transport_reference::Server, color: &str| {
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
            document.documentElement.style.background=ok?{color}:'#ff00ff';document.documentElement.dataset.transport='yes';
          }}
        }})();
        </script></body></html>"#,
                stun = json!(format!("stun:{}", server.stun)),
                turn = json!(format!("turn:{}?transport=tcp", server.tcp.address)),
                quic = json!(format!("https://{}/owned", server.quic)),
                worker_quic = json!(format!("https://{}/owned-worker", server.worker_quic)),
                color = json!(color)
            );
            fs::write(f.site().join("index.html"), html).unwrap();
        };
        let policy = |name: &str| {
            fs::write(f.temp.path().join("lazypencil.toml"),format!("[runtime.browser]\nnetwork_policy='{name}'\nready_selector=\"html[data-transport='yes']\"\nselector_state='attached'\ntimeout_ms=15000\n")).unwrap();
        };
        let mut runtime = if retained {
            f.runtime().with_retained_pages(f.store.path())
        } else {
            f.runtime()
        };
        let capture = |runtime: &mut DynamicRuntime,
                       stage: &str,
                       refresh: bool,
                       server: &transport_reference::Server| {
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
                fs::write(std::path::Path::new(&directory).join("codec-offline-transports-observation.json"),serde_json::to_vec_pretty(&json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"retained":retained,"stage":stage,"STUN_TURN_pageQUIC_workerQUIC_counts":server.counts(),"counts_by_endpoint_phase":{"online":online_sink.counts(),"offline":offline_sink.counts(),"online-resumed":resumed_sink.counts()},"error":result.as_ref().err().map(|error|json!({"code":error.code,"message":error.message}))})).unwrap()).unwrap();
            }
            result.unwrap_or_else(|error| {
                panic!(
                    "{retained}/{stage}: {error:?}; counts={:?}",
                    server.counts()
                )
            })
        };
        let mut keep = |stage: &str,
                        phase: &str,
                        image: &RenderedImage,
                        server: &transport_reference::Server,
                        rgba: [u8; 4]| {
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
                        Color::from_argb(rgba[3], rgba[0], rgba[1], rgba[2]),
                        "{retained}/{stage}/{x},{y}"
                    );
                }
            }
            let name = format!("codec-offline-transports-{phase}.png");
            if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
                let path = std::path::Path::new(&directory).join(&name);
                if path.exists() {
                    assert_eq!(fs::read(path).unwrap(), image.png);
                } else {
                    fs::create_dir_all(&directory).unwrap();
                    fs::write(path, &image.png).unwrap();
                }
            }
            assert_eq!(acquisition(image)["page_state"]["retained"], retained);
            let metadata = acquisition(image);
            rows.push(json!({"retained":retained,"stage":stage,"endpoint_phase":phase,"png":name,"png_hash":lazypencil::document::hash(&image.png),"expected_rgba":rgba,"STUN_TURN_pageQUIC_workerQUIC_counts":server.counts(),"network_policy":metadata["network_policy"],"limits_policy":metadata["limits"]["browser"]["network_policy"],"page_state":metadata["page_state"],"acquisition_id":metadata["acquisition_id"],"browser_version":metadata["browser_version"]}));
        };
        write_page(online_sink, "#00ff00");
        policy("online");
        let online = capture(&mut runtime, "online", false, online_sink);
        let first = online_sink.counts();
        assert!(
            first.0 > 0 && first.1 > 0 && first.2 > 0 && first.3 > 0,
            "All positive transport controls must reach their owned sinks: {first:?}"
        );
        assert_eq!(acquisition(&online)["network_policy"], "online");
        assert_eq!(acquisition(&online)["page_state"]["retained"], retained);
        let online_process = runtime.process_ids();
        assert_eq!(online_process.len(), 1);
        keep("online", "online", &online, online_sink, [0, 255, 0, 255]);
        write_page(offline_sink, "#0000ff");
        policy("offline");
        let offline = capture(&mut runtime, "offline", false, offline_sink);
        assert_eq!(
            offline_sink.counts(),
            (0, 0, 0, 0),
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
        keep(
            "offline",
            "offline",
            &offline,
            offline_sink,
            [0, 0, 255, 255],
        );
        let refreshed = capture(&mut runtime, "offline-refresh", true, offline_sink);
        assert_eq!(offline_sink.counts(), (0, 0, 0, 0));
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
        keep(
            "offline-refresh",
            "offline",
            &refreshed,
            offline_sink,
            [0, 0, 255, 255],
        );
        write_page(resumed_sink, "#00ffff");
        policy("online");
        let resumed = capture(&mut runtime, "online-resumed", false, resumed_sink);
        let last = resumed_sink.counts();
        assert!(
            last.0 > 0 && last.1 > 0 && last.2 > 0 && last.3 > 0,
            "Online recovery must reach all owned sinks: {last:?}"
        );
        assert_ne!(runtime.process_ids(), offline_process);
        assert_eq!(acquisition(&resumed)["network_policy"], "online");
        keep(
            "online-resumed",
            "online-resumed",
            &resumed,
            resumed_sink,
            [0, 255, 255, 255],
        );
    }
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
        fs::write(std::path::Path::new(&directory).join("codec-offline-transports.json"),serde_json::to_vec_pretty(&json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"cases":rows,"observation_ms":1500,"distinct_udp_endpoints_per_mode":9,"distinct_tcp_endpoints_per_mode":3,"distinct_udp_endpoints_total":18,"distinct_tcp_endpoints_total":6,"transport_control":"Actual STUN datagrams, TURN/TCP requests and page/worker QUIC long-header datagrams. Separate endpoints per mode and phase distinguish new acquisitions from prior context teardown; phase colors prove captured HTML freshness. Owned sinks intentionally do not complete sessions; positive controls prove routing, not successful peer/media or WebTransport content."})).unwrap()).unwrap();
    }
}

//! Router management runs outside HTTP/protocol workers. It never replaces another client's mapping.
use crate::model::{Connectivity, NetworkSettings};
use anyhow::{Context, Result, ensure};
use igd_next::{
    AddPortError, Gateway, GetGenericPortMappingEntryError, PortMappingProtocol as Protocol,
};
use std::{
    net::{IpAddr, SocketAddr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;
#[derive(Clone, PartialEq)]
struct Control {
    settings: NetworkSettings,
    active: bool,
    probe: u64,
}
pub struct Network {
    control: watch::Sender<Control>,
    pub status: watch::Receiver<Connectivity>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
struct Lease {
    gateway: Gateway,
    port: u16,
    target: SocketAddr,
    description: String,
}
#[derive(Clone)]
pub struct Mapping {
    pub port: u16,
    pub target: String,
    pub internal_port: u16,
    pub description: String,
}
pub fn belongs_to(mapping: &Mapping, port: u16, target: SocketAddr, description: &str) -> bool {
    mapping.port == port
        && mapping.internal_port == target.port()
        && mapping.target == target.ip().to_string()
        && mapping.description == description
}
fn entry(g: &Gateway, port: u16, stop: &AtomicBool) -> Result<Option<Mapping>> {
    let deadline = Instant::now() + Duration::from_secs(8);
    for i in 0..256 {
        ensure!(!stop.load(Ordering::Relaxed), "Router operation stopped");
        ensure!(
            Instant::now() < deadline,
            "Router mapping lookup took too long; use manual forwarding"
        );
        match g.get_generic_port_mapping_entry(i) {
            Ok(e) if e.protocol == Protocol::TCP && e.external_port == port => {
                return Ok(Some(Mapping {
                    port: e.external_port,
                    target: e.internal_client,
                    internal_port: e.internal_port,
                    description: e.port_mapping_description,
                }));
            }
            Ok(_) => {}
            Err(GetGenericPortMappingEntryError::SpecifiedArrayIndexInvalid) => return Ok(None),
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Cannot verify router mapping ownership: {e}"
                ));
            }
        }
    }
    anyhow::bail!("Router has too many mappings to verify ownership safely; use manual forwarding")
}
fn remove(lease: &Lease, stop: &AtomicBool) -> Result<()> {
    if let Some(e) = entry(&lease.gateway, lease.port, stop)? {
        ensure!(
            belongs_to(&e, lease.port, lease.target, &lease.description),
            "Router mapping changed owner; it was left untouched"
        );
        lease.gateway.remove_port(Protocol::TCP, lease.port)?;
    }
    Ok(())
}
fn configure(
    gateway: &Gateway,
    target: SocketAddr,
    description: &str,
    stop: &AtomicBool,
) -> Result<u32> {
    if let Some(e) = entry(gateway, target.port(), stop)? {
        ensure!(
            belongs_to(&e, target.port(), target, description),
            "That router port is already assigned to another application. Choose another transfer port"
        );
    }
    ensure!(!stop.load(Ordering::Relaxed), "Router operation stopped");
    match gateway.add_port(Protocol::TCP, target.port(), target, 3600, description) {
        Ok(()) => Ok(3600),
        Err(AddPortError::OnlyPermanentLeasesSupported) => {
            gateway.add_port(Protocol::TCP, target.port(), target, 0, description)?;
            Ok(0)
        }
        Err(e) => Err(e.into()),
    }
}
pub fn non_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1])
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}
fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
impl Network {
    pub fn start(settings: NetworkSettings, identity: String) -> Result<Self> {
        let (control, mut rx) = watch::channel(Control {
            settings,
            active: false,
            probe: 0,
        });
        let(tx,status)=watch::channel(Connectivity{state:"disabled".into(),message:"Automatic router forwarding is off. You can forward the transfer TCP port manually.".into(),..Default::default()});
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread=std::thread::Builder::new().name("router-connectivity".into()).spawn(move||{
   let mut current=rx.borrow().clone();let mut lease:Option<Lease>=None;let mut due=Instant::now();let mut probed=0;
   while !stopping.load(Ordering::Relaxed) {
    let changed=rx.has_changed().unwrap_or(false);if changed{current=rx.borrow_and_update().clone();due=Instant::now();}
    if Instant::now()<due {std::thread::sleep(Duration::from_millis(100));continue;}
    if lease.as_ref().is_some_and(|l|!current.settings.upnp||!current.active||l.port!=current.settings.port){
     if let Err(e)=remove(lease.as_ref().unwrap(),&stopping){tx.send_replace(Connectivity{state:"cleanup_failed".into(),message:format!("Could not remove this client's router rule; retrying. {e}"),checked_at:timestamp(),..Default::default()});due=Instant::now()+Duration::from_secs(60);continue;}lease=None;
    }
    let probe=current.probe!=probed;probed=current.probe;
    if !current.settings.upnp&&!probe {tx.send_replace(Connectivity{state:"disabled".into(),message:format!("Automatic forwarding is off. Forward TCP {} to this server if required.",current.settings.port),checked_at:timestamp(),..Default::default()});due=Instant::now()+Duration::from_secs(3600);continue;}
    tx.send_replace(Connectivity{state:"checking".into(),message:"Looking for a UPnP Internet Gateway on your local network…".into(),checked_at:timestamp(),..Default::default()});
    let result=(||->Result<Connectivity>{
     let mut options=igd_next::SearchOptions::default();options.timeout=Some(Duration::from_secs(4));options.single_search_timeout=Some(Duration::from_secs(2));
     let gateway=igd_next::search_gateway(options).context("No usable UPnP gateway was found. Check router UPnP settings or use manual TCP forwarding")?;
     let socket=UdpSocket::bind("0.0.0.0:0")?;socket.connect(gateway.addr)?;let local=socket.local_addr()?.ip();ensure!(local.is_ipv4(),"Soulseek incoming transfers require an IPv4 mapping");
     let external=gateway.get_external_ip()?;let mut state=Connectivity{state:"ready".into(),message:"Router found. Connect to Soulseek to enable incoming forwarding.".into(),gateway:Some(gateway.addr.ip().to_string()),external_ip:Some(external.to_string()),local_ip:Some(local.to_string()),checked_at:timestamp(),..Default::default()};
     if current.settings.upnp&&current.active {
      let target=SocketAddr::new(local,current.settings.port);let description=format!("RustySoul-{}",&identity[..identity.len().min(12)]);
      ensure!(!stopping.load(Ordering::Relaxed)&&!rx.has_changed().unwrap_or(false),"Router settings changed; retrying");
      let seconds=configure(&gateway,target,&description,&stopping)?;
      lease=Some(Lease{gateway,port:current.settings.port,target,description});state.state="mapped".into();state.mapped=true;state.lease_seconds=seconds;state.message="Incoming TCP forwarding is configured and will be renewed automatically. Internet reachability has not been independently verified.".into();
     }
     if non_public(external){state.message.push_str(" The router reports a non-public external address; upstream NAT/CGNAT may prevent incoming connections.");}
     Ok(state)
    })();
    match result {Ok(state)=>{tx.send_replace(state);due=Instant::now()+Duration::from_secs(if current.active&&current.settings.upnp{1200}else{300});},Err(e)=>{tx.send_replace(Connectivity{state:"unavailable".into(),message:e.to_string(),checked_at:timestamp(),..Default::default()});due=Instant::now()+Duration::from_secs(60);}}
   }
   if let Some(lease)=lease {let cleanup=AtomicBool::new(false);if let Err(e)=remove(&lease,&cleanup){tracing::warn!(error=%e,"Could not remove RustySoul UPnP mapping on shutdown");}}
  })?;
        Ok(Self {
            control,
            status,
            stop,
            thread: Some(thread),
        })
    }
    pub fn update(&self, settings: NetworkSettings, active: bool) {
        let old = self.control.borrow().clone();
        let next = Control {
            settings,
            active,
            probe: old.probe,
        };
        if old != next {
            self.control.send_replace(next);
        }
    }
    pub fn check(&self) {
        self.control.send_modify(|v| v.probe += 1);
    }
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
impl Drop for Network {
    fn drop(&mut self) {
        self.shutdown();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::Mutex;
    struct MockGateway {
        gateway: Gateway,
        mapping: Arc<Mutex<Option<String>>>,
        calls: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl MockGateway {
        fn new(permanent: bool) -> Self {
            let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = server.local_addr().unwrap();
            server.set_nonblocking(true).unwrap();
            let mapping: Arc<Mutex<Option<String>>> = Arc::default();
            let calls: Arc<Mutex<Vec<String>>> = Arc::default();
            let stop = Arc::new(AtomicBool::new(false));
            let (m, c, s) = (mapping.clone(), calls.clone(), stop.clone());
            let thread = std::thread::spawn(move || {
                while !s.load(Ordering::Relaxed) {
                    let Ok((mut stream, _)) = server.accept() else {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = Vec::new();
                    let mut byte = [0];
                    while !request.ends_with(b"\r\n\r\n") {
                        stream.read_exact(&mut byte).unwrap();
                        request.push(byte[0]);
                    }
                    let headers = String::from_utf8(request).unwrap();
                    let length: usize = headers
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|n| n.trim().parse().unwrap())
                        })
                        .unwrap();
                    let mut body = vec![0; length];
                    stream.read_exact(&mut body).unwrap();
                    let body = String::from_utf8(body).unwrap();
                    let (action, response) = if body.contains("GetGenericPortMappingEntry") {
                        (
                            "GetGenericPortMappingEntry",
                            match m.lock().unwrap().as_ref() {
                                Some(description) => format!(
                                    "<u:GetGenericPortMappingEntryResponse xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\"><NewRemoteHost></NewRemoteHost><NewExternalPort>2234</NewExternalPort><NewProtocol>TCP</NewProtocol><NewInternalPort>2234</NewInternalPort><NewInternalClient>192.168.1.1</NewInternalClient><NewEnabled>1</NewEnabled><NewPortMappingDescription>{description}</NewPortMappingDescription><NewLeaseDuration>3600</NewLeaseDuration></u:GetGenericPortMappingEntryResponse>"
                                ),
                                None => fault(713),
                            },
                        )
                    } else if body.contains("AddPortMapping") {
                        if permanent && body.contains("<NewLeaseDuration>3600</NewLeaseDuration>") {
                            ("AddPortMapping:3600", fault(725))
                        } else {
                            let description = body
                                .split("<NewPortMappingDescription>")
                                .nth(1)
                                .unwrap()
                                .split("</")
                                .next()
                                .unwrap()
                                .to_string();
                            *m.lock().unwrap() = Some(description);
                            (if permanent {"AddPortMapping:0"}else{"AddPortMapping:3600"}, "<u:AddPortMappingResponse xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\"/>".into())
                        }
                    } else if body.contains("DeletePortMapping") {
                        *m.lock().unwrap() = None;
                        ("DeletePortMapping", "<u:DeletePortMappingResponse xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\"/>".into())
                    } else {
                        panic!("Unexpected SOAP action: {body}");
                    };
                    c.lock().unwrap().push(action.into());
                    let xml = format!(
                        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body>{response}</s:Body></s:Envelope>"
                    );
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",xml.len(),xml).unwrap();
                }
            });
            let schema = [
                "NewRemoteHost",
                "NewExternalPort",
                "NewProtocol",
                "NewInternalPort",
                "NewInternalClient",
                "NewEnabled",
                "NewPortMappingDescription",
                "NewLeaseDuration",
            ]
            .map(String::from)
            .to_vec();
            Self {
                gateway: Gateway {
                    addr,
                    root_url: "/".into(),
                    control_url: "/control".into(),
                    control_schema_url: "/schema".into(),
                    control_schema: [
                        ("AddPortMapping".into(), schema),
                        (
                            "DeletePortMapping".into(),
                            ["NewRemoteHost", "NewExternalPort", "NewProtocol"]
                                .map(String::from)
                                .to_vec(),
                        ),
                    ]
                    .into(),
                    service_type: "urn:schemas-upnp-org:service:WANIPConnection:1".into(),
                },
                mapping,
                calls,
                stop,
                thread: Some(thread),
            }
        }
    }
    fn fault(code: u16) -> String {
        format!(
            "<s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns=\"urn:schemas-upnp-org:control-1-0\"><errorCode>{code}</errorCode><errorDescription>fixture</errorDescription></UPnPError></detail></s:Fault>"
        )
    }
    impl Drop for MockGateway {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            self.thread.take().unwrap().join().unwrap();
        }
    }
    #[test]
    fn soap_mapping_creation_renewal_ownership_and_cleanup() {
        let mock = MockGateway::new(false);
        let stop = AtomicBool::new(false);
        let target = "192.168.1.1:2234".parse().unwrap();
        assert_eq!(
            configure(&mock.gateway, target, "ours", &stop).unwrap(),
            3600
        );
        assert_eq!(
            configure(&mock.gateway, target, "ours", &stop).unwrap(),
            3600
        );
        let lease = Lease {
            gateway: mock.gateway.clone(),
            port: 2234,
            target,
            description: "ours".into(),
        };
        *mock.mapping.lock().unwrap() = Some("someone else".into());
        assert!(configure(&mock.gateway, target, "ours", &stop).is_err());
        assert!(remove(&lease, &stop).is_err());
        assert!(
            !mock
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c == "DeletePortMapping")
        );
        *mock.mapping.lock().unwrap() = Some("ours".into());
        remove(&lease, &stop).unwrap();
        assert!(mock.mapping.lock().unwrap().is_none());
        remove(&lease, &stop).unwrap();
    }
    #[test]
    fn soap_permanent_lease_fallback_and_stopped_operations() {
        let mock = MockGateway::new(true);
        let stop = AtomicBool::new(false);
        assert_eq!(
            configure(
                &mock.gateway,
                "192.168.1.1:2234".parse().unwrap(),
                "ours",
                &stop
            )
            .unwrap(),
            0
        );
        assert!(
            mock.calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c == "AddPortMapping:0")
        );
        stop.store(true, Ordering::Relaxed);
        assert!(entry(&mock.gateway, 2234, &stop).is_err());
    }
    #[test]
    fn never_claim_another_mapping() {
        let target = "192.168.1.1:2234".parse().unwrap();
        let mut m = Mapping {
            port: 2234,
            target: "192.168.1.1".into(),
            internal_port: 2234,
            description: "ours".into(),
        };
        assert!(belongs_to(&m, 2234, target, "ours"));
        m.description = "someone else".into();
        assert!(!belongs_to(&m, 2234, target, "ours"));
        m.description = "ours".into();
        m.target = "192.168.1.2".into();
        assert!(!belongs_to(&m, 2234, target, "ours"));
    }
    #[test]
    fn detect_upstream_private_and_shared_nat() {
        for ip in ["192.168.1.1", "10.0.0.1", "100.64.0.1", "100.127.255.255"] {
            assert!(non_public(ip.parse().unwrap()));
        }
        assert!(!non_public("8.8.8.8".parse().unwrap()));
    }
}

use super::{
    ActiveUpload, Arc, BrokeredUpload, Client, ClientContext, DownloadStatus,
    PeerMessage, RwLock, RwLockExt, ServerMessage, collect_failed_tokens,
    debug, error, warn, next_connect_token, next_upload_token, thread,
};
use crate::message::server::MessageFactory;
use crate::peer::{ConnectionType, upload_peer};
use crate::types::UploadStatus;
use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

impl Client {
    /// None measures normally; a positive value overrides it; zero withholds speed.
    /// Configuration alone sends no server report and does not inflate upload counts.
    pub fn configure_upload_speed_reporting(&self, override_speed:Option<u32>, measured:u32) {
        if let Ok(mut ctx)=self.context.write_safe() {ctx.reported_upload_speed=override_speed;ctx.measured_upload_speed=Some(measured);}
    }
    pub fn own_upload_availability(&self)->(u32,bool) {
        self.context.read_safe().map(|ctx|(ctx.upload_queue.len() as u32,ctx.has_free_upload_slot())).unwrap_or((0,false))
    }
    /// Local manual preferences apply within the protocol's privilege classes;
    /// already offered/streaming transfers are not interrupted.
    pub fn set_upload_queue_order(&self, rules: Vec<(String,String,i8,u64)>) {
        if let Ok(mut ctx)=self.context.write_safe() {
            ctx.upload_order=rules.into_iter().take(32768).map(|(u,p,priority,position)|((u.to_lowercase(),p),(priority.clamp(-1,1),position))).collect();
            let order=ctx.upload_order.clone();
            for job in &mut ctx.upload_queue { job.manual_order=order.get(&(job.downloader.to_lowercase(),job.virtual_path.clone())).copied(); }
        }
        Self::pump_upload_queue(&self.context);
    }

    /// Atomically cancel this user's queued, offered and streaming uploads.
    /// An empty path list means all their current requests, never other users.
    pub fn cancel_uploads(&self,username:&str,paths:&[String])->Vec<crate::types::UploadInfo>{
        let matches=|u:&str,p:&str|u==username&&(paths.is_empty()||paths.iter().any(|path|path==p));
        let (cancelled,registry)=if let Ok(mut ctx)=self.context.write_safe(){
            let mut cancelled=std::collections::BTreeMap::<String,crate::types::UploadInfo>::new();
            for mut info in ctx.queued_uploads(){if matches(&info.username,&info.filename){info.status=UploadStatus::Cancelled;cancelled.insert(info.filename.clone(),info);}}
            ctx.upload_queue.retain(|u|!matches(&u.downloader,&u.virtual_path));
            ctx.uploads.retain(|_,u|{if !matches(&u.downloader,&u.virtual_path){return true;}cancelled.insert(u.virtual_path.clone(),crate::types::UploadInfo{username:u.downloader.clone(),filename:u.virtual_path.clone(),size:u.size,bytes_sent:0,speed_bytes_per_sec:0.,status:UploadStatus::Cancelled});false});
            for active in ctx.active_uploads.values_mut().filter(|u|matches(&u.username,&u.filename)&&matches!(u.status,UploadStatus::InProgress)){
                active.cancel.store(true,Ordering::Relaxed);active.status=UploadStatus::Cancelled;cancelled.insert(active.filename.clone(),crate::types::UploadInfo{username:active.username.clone(),filename:active.filename.clone(),size:active.size,bytes_sent:active.bytes_sent.load(Ordering::Relaxed),speed_bytes_per_sec:0.,status:UploadStatus::Cancelled});
            }
            let cancelled=cancelled.into_values().collect::<Vec<_>>();ctx.upload_events.extend(cancelled.iter().cloned());let overflow=ctx.upload_events.len().saturating_sub(256);ctx.upload_events.drain(..overflow);(cancelled,ctx.peer_registry.clone())
        }else{return Vec::new();};
        if let Some(registry)=registry{for u in &cancelled{let message=crate::message::Message::new().write_int32(50).write_string(&u.filename).write_string("Cancelled").clone();let _=registry.send_to_peer(username,PeerMessage::SendMessage(message));}}
        Self::pump_upload_queue(&self.context);cancelled
    }
    /// Deny these peers share listings and new uploads; cancel any offered or
    /// running transfers. The legacy protocol does not cryptographically authenticate names.
    pub fn set_banned_users(&self,users:Vec<String>) {
        if let Ok(mut ctx)=self.context.write_safe(){
            ctx.banned_users=users.into_iter().map(|u|u.to_ascii_lowercase()).collect();
            let users:Vec<_>=ctx.banned_users.iter().cloned().collect();
            for user in users {ctx.release_upload_slots(&user);for active in ctx.active_uploads.values_mut().filter(|u|u.username.eq_ignore_ascii_case(&user)&&matches!(u.status,crate::types::UploadStatus::InProgress)){active.cancel.store(true,std::sync::atomic::Ordering::Relaxed);active.status=crate::types::UploadStatus::Cancelled;}}
        }
        Self::pump_upload_queue(&self.context);
    }
    pub(super) fn cancel_upload_request(&self,username:&str,filename:&str)->bool {
        let mut changed=false;
        if let Ok(mut ctx)=self.context.write_safe(){
            let before=ctx.upload_queue.len();ctx.upload_queue.retain(|u|u.downloader!=username||u.virtual_path!=filename);changed|=before!=ctx.upload_queue.len();
            ctx.uploads.retain(|_,u|{let remove=u.downloader==username&&u.virtual_path==filename;changed|=remove;!remove});
            for u in ctx.active_uploads.values_mut().filter(|u|u.username==username&&u.filename==filename&&matches!(u.status,crate::types::UploadStatus::InProgress)){u.cancel.store(true,std::sync::atomic::Ordering::Relaxed);u.status=crate::types::UploadStatus::Cancelled;changed=true;}
        }
        if changed{Self::pump_upload_queue(&self.context);}changed
    }
    /// Forget finished upload records only. Active sockets and files are never touched.
    pub fn clear_upload_history(&self, status:&str)->usize {
        let matches=|s:&UploadStatus|match s {UploadStatus::Completed=>status=="all"||status=="completed",UploadStatus::Cancelled=>status=="all"||status=="cancelled",UploadStatus::Failed(_)=>status=="all"||status=="failed",UploadStatus::TimedOut(_)=>status=="all"||status=="timed_out",_=>false};
        if let Ok(mut ctx)=self.context.write_safe(){let before=ctx.active_uploads.len();ctx.active_uploads.retain(|_,u|!matches(&u.status));ctx.upload_events.retain(|u|!matches(&u.status));before-ctx.active_uploads.len()}else{0}
    }
    /// Whether the server listed `username` as privileged. Privileged peers are
    /// served before everyone else.
    #[must_use]
    pub fn is_privileged(&self, username: &str) -> bool {
        self.context
            .read_safe()
            .is_ok_and(|ctx| ctx.is_privileged(username))
    }

    /// Ask the server how much of our own privilege time is left (code 92). The
    /// answer arrives asynchronously; read it with
    /// [`Client::own_privilege_seconds`].
    ///
    /// # Errors
    /// [`crate::error::SoulseekRs::NotConnected`] without a server connection.
    pub fn check_privileges(&self) -> crate::error::Result<()> {
        let Some(handle) = &self.server_handle else {
            return Err(crate::error::SoulseekRs::NotConnected);
        };
        handle.send(super::ServerMessage::CheckPrivileges).map_err(|_| crate::error::SoulseekRs::NotConnected)?;
        Ok(())
    }

    /// Seconds of our own privileges left, or `None` until the server has
    /// answered a [`Client::check_privileges`].
    #[must_use]
    pub fn own_privilege_seconds(&self) -> Option<u32> {
        self.context
            .read_safe()
            .ok()
            .and_then(|ctx| ctx.own_privileges)
    }

    /// Latest privilege seconds and a revision that changes for every server reply,
    /// including repeated zero values. The revision is local to this client instance.
    #[must_use]
    pub fn own_privilege_observation(&self) -> Option<(u32, u64)> {
        self.context.read_safe().ok().and_then(|ctx| {
            ctx.own_privileges.map(|seconds| (seconds, ctx.own_privileges_revision))
        })
    }

    /// Queued-upload states recorded since the last call, for a caller that
    /// samples [`Client::uploads`] and would otherwise miss a peer that queued
    /// and was served between two samples.
    #[must_use]
    pub fn take_upload_events(&self) -> Vec<crate::types::UploadInfo> {
        self.context
            .write_safe()
            .map(|mut ctx| ctx.take_upload_events())
            .unwrap_or_default()
    }

    /// How many uploads run at once. Lowering it does not interrupt transfers
    /// already in flight; the queue simply refills more slowly.
    pub fn set_upload_slots(&self, slots: usize) {
        if let Ok(mut ctx) = self.context.write_safe() {
            ctx.upload_slots = slots;
        }
        Self::pump_upload_queue(&self.context);
    }
    pub fn set_upload_slots_per_user(&self, slots:usize) {
        if let Ok(mut ctx)=self.context.write_safe(){ctx.upload_slots_per_user=slots.max(1);}
        Self::pump_upload_queue(&self.context);
    }
    pub fn set_upload_limits(&self,total:usize,per_user:usize) {
        if let Ok(mut ctx)=self.context.write_safe(){ctx.upload_slots=total;ctx.upload_slots_per_user=per_user.max(1);}
        Self::pump_upload_queue(&self.context);
    }

    /// Drop everything `username` was queued or offered, then hand their slot to
    /// whoever is next.
    pub(crate) fn release_upload_slots(
        client_context: &Arc<RwLock<ClientContext>>,
        username: &str,
    ) {
        let freed = client_context
            .write_safe()
            .is_ok_and(|mut ctx| ctx.release_upload_slots(username));
        if freed {
            Self::pump_upload_queue(client_context);
        }
    }

    /// Fill every free upload slot from the queue and send the offers.
    ///
    /// Called whenever the queue or the slot count could have changed: a new
    /// request, a finished transfer, or a privileged list that re-ranked who is
    /// waiting.
    pub(crate) fn pump_upload_queue(
        client_context: &Arc<RwLock<ClientContext>>,
    ) {
        let (registry, offers,denials) = match client_context.write_safe() {
            Ok(mut ctx) => {let (registry,offers)=ctx.pump_uploads(next_upload_token);let denials=std::mem::take(&mut ctx.sharing_denials);(registry,offers,denials)},
            Err(e) => {
                error!("[client] pump_upload_queue write: {}", e);
                return;
            }
        };
        let Some(registry) = registry else {
            return;
        };
        for (user,path,reason) in denials{let message=crate::message::Message::new().write_int32(50).write_string(&path).write_string(&reason).clone();let _=registry.send_to_peer(&user,PeerMessage::SendMessage(message));}
        for offer in offers {
            let _ = registry.send_to_peer(
                &offer.requester_key,
                PeerMessage::ServeUpload {
                    token: offer.token,
                    filename: offer.virtual_path,
                    size: offer.size,
                },
            );
        }
    }

    /// Consume the upload job for `token` and stream the file to `host:port`
    /// on a background thread.
    pub(crate) fn spawn_serve(
        client_context: &Arc<RwLock<ClientContext>>,
        own_username: &str,
        token: u32,
        host: String,
        port: u32,
    ) {
        let Ok(mut ctx) = client_context.write_safe() else {
            return;
        };
        let Some(job) = ctx.uploads.remove(&token) else {
            return;
        };
        let bytes_sent = Arc::new(AtomicU64::new(0));
        let bandwidth=ctx.upload_speed_limit.for_user(&job.downloader);
        let cancel = Arc::new(AtomicBool::new(false));
        ctx.active_uploads.insert(
            token,
            ActiveUpload {
                payload_bytes: bandwidth.transferred.clone(),
                username: job.downloader.clone(),
                filename: job.virtual_path.clone(),
                size: job.size,
                bytes_sent: bytes_sent.clone(),
                cancel: cancel.clone(),
                status: UploadStatus::InProgress,
                started: std::time::Instant::now(),
            },
        );
        drop(ctx);
        let own = own_username.to_string();
        let real_path = job.real_path;
        let downloader = job.downloader;
        let context = client_context.clone();
        thread::spawn(move || {
            // Most downloaders sit behind a NAT, so dialling them is a gamble
            // that costs a 20s timeout when it fails. Like the reference
            // clients, ask the server to have them connect back at the same
            // time; the first socket to arrive claims the transfer, and port 0
            // (the server not knowing where they listen) skips the dial.
            let Some((connect_token, claimed)) =
                Self::broker_upload(&context, token, &real_path, &downloader)
            else {
                return;
            };
            if port == 0 {
                return;
            }
            let stream = match upload_peer::dial(&host, port) {
                Ok(stream) => stream,
                Err(e) => {
                    debug!(
                        "[client] cannot dial {downloader} at {host}:{port} \
                         for upload {token} ({e}); waiting for them to connect"
                    );
                    return;
                }
            };
            if claimed
                .compare_exchange(
                    false,
                    true,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
            {
                return;
            }
            if let Ok(mut ctx) = context.write_safe() {
                ctx.take_brokered_upload(connect_token);
            }
            let result = upload_peer::serve_direct_limited(
                stream,
                &own,
                token,
                &real_path,
                &bytes_sent,
                &cancel,
                Some(&bandwidth),
            );
            Self::finish_upload(&context, token, &real_path, result);
        });
    }

    /// Park the upload under a correlation token and ask the server to have
    /// `downloader` connect back to us with it. Returns the token and the claim
    /// flag the dialling route races against; `None` fails the upload.
    fn broker_upload(
        context: &Arc<RwLock<ClientContext>>,
        token: u32,
        real_path: &Path,
        downloader: &str,
    ) -> Option<(u32, Arc<AtomicBool>)> {
        let connect_token = next_connect_token();
        let (claimed, sender) = match context.write_safe() {
            Ok(mut ctx) => (
                ctx.add_brokered_upload(
                    connect_token,
                    token,
                    real_path.to_path_buf(),
                ),
                ctx.server_sender.clone(),
            ),
            Err(_) => return None,
        };
        let Some(sender) = sender else {
            Self::fail_upload(context, token, "not connected to the server");
            return None;
        };
        let _ = sender.send(ServerMessage::SendMessage(
            MessageFactory::build_connect_to_peer(
                connect_token,
                downloader,
                ConnectionType::F,
            ),
        ));
        Some((connect_token, claimed))
    }

    /// The downloader of a brokered upload connected back: stream the file
    /// over its socket on a background thread, unless the dial beat it.
    pub(crate) fn serve_pierced(
        client_context: &Arc<RwLock<ClientContext>>,
        upload: BrokeredUpload,
        stream: TcpStream,
    ) {
        if !upload.claim() {
            return;
        }
        let Some((bytes_sent, cancel, bandwidth)) =
            client_context.write_safe().ok().and_then(|mut ctx| {
                let active = ctx.active_uploads.get_mut(&upload.token)?;
                active.started = std::time::Instant::now();
                let name=active.username.clone();let sent=active.bytes_sent.clone();let cancel=active.cancel.clone();
                let bandwidth=ctx.upload_speed_limit.for_user(&name);
                ctx.active_uploads.get_mut(&upload.token)?.payload_bytes=bandwidth.transferred.clone();
                Some((sent,cancel,bandwidth))
            })
        else {
            return;
        };
        let context = client_context.clone();
        thread::spawn(move || {
            let result = upload_peer::serve_pierced_limited(
                stream,
                upload.token,
                &upload.real_path,
                &bytes_sent,
                &cancel,
                Some(&bandwidth),
            );
            Self::finish_upload(
                &context,
                upload.token,
                &upload.real_path,
                result,
            );
        });
    }

    /// Mark upload `token` failed and hand its slot to the next in line.
    pub(crate) fn fail_upload(
        client_context: &Arc<RwLock<ClientContext>>,
        token: u32,
        reason: &str,
    ) {
        Self::record_upload_failure(client_context, token, UploadStatus::Failed(reason.to_string()));
    }
    pub(crate) fn timeout_upload(context: &Arc<RwLock<ClientContext>>, token: u32, reason: &str) {
        Self::record_upload_failure(context, token, UploadStatus::TimedOut(reason.to_string()));
    }
    fn record_upload_failure(context: &Arc<RwLock<ClientContext>>, token: u32, status: UploadStatus) {
        if let Ok(mut ctx) = context.write_safe()
            && let Some(upload) = ctx.active_uploads.get_mut(&token)
            && matches!(upload.status,UploadStatus::InProgress)
        {
            if upload.cancel.load(Ordering::Relaxed) {
                upload.status=UploadStatus::Cancelled;
            } else {
            match &status {
                UploadStatus::TimedOut(reason) => warn!("[client] upload {token} to {}: {reason}",upload.username),
                UploadStatus::Failed(reason) => error!("[client] upload {token} to {}: {reason}",upload.username),
                _ => {},
            }
            upload.status = status;
            }
        }
        Self::pump_upload_queue(context);
    }

    /// Record how a served upload ended, report its rate to the server and free
    /// its slot.
    fn finish_upload(
        context: &Arc<RwLock<ClientContext>>,
        token: u32,
        real_path: &Path,
        result: std::io::Result<u64>,
    ) {
        let streamed = result.as_ref().ok().copied();
        let mut completed_now = false;
        let mut report = None;
        if let Ok(mut ctx) = context.write_safe()
            && let Some(upload) = ctx.active_uploads.get_mut(&token)
        {
            // UI cancellation and a late socket error may race. Cancellation
            // already acknowledged to the user must remain terminal and quiet.
            if upload.cancel.load(Ordering::Relaxed) || matches!(upload.status,UploadStatus::Cancelled) {
                upload.status=UploadStatus::Cancelled;
            } else if matches!(upload.status,UploadStatus::InProgress) {
                upload.status=match &result {
                    Ok(_) => { completed_now = true; UploadStatus::Completed },
                    Err(e) if e.kind()==std::io::ErrorKind::Interrupted => UploadStatus::Cancelled,
                    Err(e) if e.kind()==std::io::ErrorKind::TimedOut => {
                        let reason=e.to_string();
                        warn!("[client] upload {}: {reason}",real_path.display());
                        UploadStatus::TimedOut(reason)
                    }
                    Err(e) if matches!(e.kind(),std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::WriteZero) => {
                        let reason="The receiving peer disconnected before the upload finished.".to_owned();
                        warn!("[client] upload {}: {reason}",real_path.display());
                        UploadStatus::Failed(reason)
                    }
                    Err(e) => {
                        error!("[client] serve {}: {}",real_path.display(),e);
                        UploadStatus::Failed(e.to_string())
                    }
                };
            }
            let secs = upload.started.elapsed().as_secs_f64();
            if let Some(bytes) = streamed && secs>0.0 && completed_now {
                let speed=(bytes as f64/secs) as u32;ctx.last_upload_speed=speed;
                let advertised=ctx.advertised_upload_speed();
                report=ctx.server_sender.clone().map(|s|(s,advertised));
            }
        }
        // The server folds each finished upload's rate into the average
        // speed it shows other users for us.
        if let Some((server, speed)) = report {
            let _ = server.send(ServerMessage::SendMessage(
                MessageFactory::build_send_upload_speed(speed),
            ));
        }
        // The slot this transfer held is free now, so whoever is next in
        // line gets it without waiting for another request to arrive.
        Self::pump_upload_queue(context);
    }

    pub(crate) fn process_failed_uploads(
        client_context: Arc<RwLock<ClientContext>>,
        username: &str,
        filename: Option<&str>,
    ) {
        let failed_tokens = match client_context.read_safe() {
            Ok(context) => {
                collect_failed_tokens(&context.downloads, username, filename)
            }
            Err(e) => {
                error!("[client] process_failed_uploads read: {}", e);
                return;
            }
        };

        if failed_tokens.is_empty() {
            return;
        }

        match client_context.write_safe() {
            Ok(mut context) => {
                for token in failed_tokens {
                    context.downloads.update_status(
                        token,
                        DownloadStatus::Failed(Some(
                            "The upload failed on the other side".to_string(),
                        )),
                    );
                    // Keep the terminal state until the daemon records it (and can schedule a retry).
                }
            }
            Err(e) => {
                error!("[client] process_failed_uploads write: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod outcomes {
    use super::*;
    #[test]
    fn configured_rates_are_reported_once_per_completed_upload_including_hidden_zero() {
        for (override_speed,expected) in [(None,123456),(Some(555000),555000),(Some(0),0)] {
            let client=client_with_upload(UploadStatus::InProgress);
            let (tx,rx)=std::sync::mpsc::channel();
            client.context.write().unwrap().server_sender=Some(tx);
            client.configure_upload_speed_reporting(override_speed,123456);
            assert!(rx.try_recv().is_err(),"Configuration must not add phantom uploaded files");
            assert_eq!(client.context.read().unwrap().advertised_upload_speed(),expected);
            Client::finish_upload(&client.context,7,Path::new("a.flac"),Ok(10));
            let ServerMessage::SendMessage(message)=rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap() else {panic!("expected rate report")};
            assert_eq!(message,MessageFactory::build_send_upload_speed(expected));
            Client::finish_upload(&client.context,7,Path::new("a.flac"),Ok(10));
            assert!(rx.try_recv().is_err(),"Duplicate completion must not count twice");
            client.configure_upload_speed_reporting(None,321000);
            assert_eq!(client.context.read().unwrap().advertised_upload_speed(),321000);
        }
    }
    #[test]
    fn own_queue_statistics_cover_the_entire_queue_without_preview_limits() {
        let client=Client::new("fixture","password");
        assert_eq!(client.own_upload_availability().0,0);
        {
            let mut ctx=client.context.write().unwrap();
            for seq in 0..900 {
                ctx.upload_queue.push(super::super::upload_queue::QueuedUpload {
                    downloader:"alice".into(),virtual_path:format!("{seq}.flac"),real_path:Path::new("unused").into(),size:10,privileged:false,seq,manual_order:None
                });
            }
        }
        assert_eq!(client.own_upload_availability().0,900);
    }
    fn client_with_upload(status:UploadStatus)->Client {
        let client=Client::new("fixture","password");
        client.context.write().unwrap().active_uploads.insert(7,ActiveUpload{payload_bytes:Arc::new(AtomicU64::new(0)),username:"alice".into(),filename:"a.flac".into(),size:10,bytes_sent:Arc::new(AtomicU64::new(4)),cancel:Arc::new(AtomicBool::new(false)),status,started:std::time::Instant::now()});client
    }
    #[test]
    fn a_late_socket_error_or_broker_timeout_cannot_overwrite_acknowledged_cancellation() {
        let client=client_with_upload(UploadStatus::InProgress);assert_eq!(client.cancel_uploads("alice",&[]).len(),1);
        Client::finish_upload(&client.context,7,Path::new("a.flac"),Err(std::io::ErrorKind::ConnectionReset.into()));
        Client::timeout_upload(&client.context,7,"Late timeout");
        assert!(matches!(client.uploads()[0].status,UploadStatus::Cancelled));assert_eq!(client.uploads()[0].bytes_sent,4);
    }
    #[test]
    fn timed_out_uploads_can_be_cleared_by_type_and_disconnections_have_useful_details() {
        let client=client_with_upload(UploadStatus::InProgress);
        Client::finish_upload(&client.context,7,Path::new("a.flac"),Err(std::io::Error::new(std::io::ErrorKind::TimedOut,"The receiving peer did not send its resume offset within 30 seconds.")));
        assert!(matches!(&client.uploads()[0].status,UploadStatus::TimedOut(reason) if reason.contains("resume offset")));assert_eq!(client.uploads()[0].speed_bytes_per_sec,0.);
        assert_eq!(client.clear_upload_history("failed"),0);assert_eq!(client.clear_upload_history("timed_out"),1);assert!(client.uploads().is_empty());
        let client=client_with_upload(UploadStatus::InProgress);Client::finish_upload(&client.context,7,Path::new("a.flac"),Err(std::io::ErrorKind::ConnectionReset.into()));
        assert!(matches!(&client.uploads()[0].status,UploadStatus::Failed(reason) if reason=="The receiving peer disconnected before the upload finished."));
    }
    #[test]
    fn genuine_local_file_errors_remain_failures_and_success_stays_completed() {
        let client=client_with_upload(UploadStatus::InProgress);Client::finish_upload(&client.context,7,Path::new("a.flac"),Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied,"Library file not readable")));
        assert!(matches!(&client.uploads()[0].status,UploadStatus::Failed(reason) if reason=="Library file not readable"));
        let client=client_with_upload(UploadStatus::InProgress);Client::finish_upload(&client.context,7,Path::new("a.flac"),Ok(10));assert!(matches!(client.uploads()[0].status,UploadStatus::Completed));
    }
}

use super::{Client,ClientContext,Duration,Instant,RwLockExt,ServerMessage};
use std::collections::HashSet;

/// 0: off, 1: shared-library requirements, 2: buddies only.
#[derive(Clone,Default)]
pub struct UploadPolicy {pub mode:u8,pub min_files:u32,pub min_folders:u32,pub buddies:HashSet<String>,pub exceptions:HashSet<String>,pub exempt_buddies:bool,pub allow_unknown:bool,pub check_seconds:u64,pub cache_seconds:u64}
pub(super) enum Decision {Allow,Checking,Deny(String)}
impl ClientContext {
    pub(super) fn upload_decision(&self,user:&str)->Decision {
        let p=&self.upload_policy;let key=user.to_ascii_lowercase();let buddy=p.buddies.contains(&key);
        if p.mode==0||p.exceptions.contains(&key)||(buddy&&p.exempt_buddies){return Decision::Allow;}
        if p.mode==2&&!buddy{return Decision::Deny("Uploads are restricted to buddies. Contact this user if you need an exception.".into());}
        if let Some(info)=self.user_info(user) {
            if info.exists==Some(false){return Decision::Deny("The server could not verify this Soulseek account.".into());}
            if self.sharing_stats_at.get(user).is_some_and(|t|t.elapsed()<Duration::from_secs(p.cache_seconds))&&let Some(stats)=info.stats {
                if stats.shared_files>=p.min_files&&stats.shared_folders>=p.min_folders{return Decision::Allow;}
                return Decision::Deny(format!("Please share at least {} file(s) in {} folder(s). Your client reports {} file(s) in {} folder(s). Retry after updating your shares.",p.min_files,p.min_folders,stats.shared_files,stats.shared_folders));
            }
        }
        if self.sharing_checks.get(user).is_some_and(|t|t.elapsed()>=Duration::from_secs(p.check_seconds)){
            if p.allow_unknown{Decision::Allow}else{Decision::Deny("Sharing could not be verified before the check timed out. Please retry when your shared-file count is available.".into())}
        }else{Decision::Checking}
    }
    pub(super) fn begin_sharing_check(&mut self,user:&str){
        if !matches!(self.upload_decision(user),Decision::Checking)||self.sharing_checks.contains_key(user){return;}
        self.sharing_checks.insert(user.into(),Instant::now());
        if let Some(sender)=&self.server_sender {let _=sender.send(ServerMessage::SendMessage(crate::message::server::MessageFactory::build_watch_user(user)));let _=sender.send(ServerMessage::SendMessage(crate::message::server::MessageFactory::build_get_user_stats(user)));}
    }
}
impl Client {
    pub fn set_upload_policy(&self,policy:UploadPolicy){if let Ok(mut c)=self.context.write_safe(){c.upload_policy=policy;let users:Vec<_>=c.upload_queue.iter().map(|q|q.downloader.clone()).collect();for user in users{c.begin_sharing_check(&user);}}Self::pump_upload_queue(&self.context);}
}

#[cfg(test)]mod tests {use super::*;use crate::client::upload_queue::QueuedUpload;
#[test]fn waiting_checks_do_not_block_verified_users_and_can_refresh_stale_counts(){
 let mut c=ClientContext::new();c.upload_policy=UploadPolicy{mode:1,min_files:1,check_seconds:5,cache_seconds:300,..Default::default()};c.apply_user_stats("alice".into(),0,10,1);c.sharing_stats_at.insert("alice".into(),Instant::now()-Duration::from_secs(301));c.enqueue_upload("alice","a.flac","a.flac".into(),100);c.apply_user_stats("bob".into(),0,10,1);c.enqueue_upload("bob","b.flac","b.flac".into(),100);let (_,offers)=c.pump_uploads(||10);assert_eq!(offers.len(),1);assert_eq!(offers[0].requester_key,"bob");assert!(c.sharing_checks.contains_key("alice"));c.apply_user_stats("alice".into(),0,10,1);let (_,offers)=c.pump_uploads(||11);assert_eq!(offers[0].requester_key,"alice");
}
#[test]fn zero_shares_are_declined_checks_do_not_take_slots_and_a_sharing_user_can_retry(){
 let mut c=ClientContext::new();c.upload_policy=UploadPolicy{mode:1,min_files:1,check_seconds:5,cache_seconds:300,..Default::default()};
 c.enqueue_upload("alice","a.flac","a.flac".into(),100);let (_,offers)=c.pump_uploads(||1);assert!(offers.is_empty());assert!(matches!(c.queued_uploads()[0].status,crate::types::UploadStatus::CheckingShares(_)));
 c.apply_user_stats("alice".into(),0,0,0);let (_,offers)=c.pump_uploads(||1);assert!(offers.is_empty());assert!(c.upload_queue.is_empty());assert!(c.sharing_denials[0].2.contains("at least 1"));
 c.apply_user_stats("alice".into(),0,10,2);c.enqueue_upload("alice","a.flac","a.flac".into(),100);let (_,offers)=c.pump_uploads(||2);assert_eq!(offers.len(),1);
}
#[test]fn buddy_exemptions_buddy_only_mode_and_timeout_choices_are_explicit(){
 let mut c=ClientContext::new();c.upload_policy=UploadPolicy{mode:2,min_files:1,check_seconds:5,cache_seconds:300,exempt_buddies:true,buddies:HashSet::from(["friend".into()]),..Default::default()};assert!(matches!(c.upload_decision("FRIEND"),Decision::Allow));assert!(matches!(c.upload_decision("other"),Decision::Deny(_)));
 c.upload_policy.mode=1;c.sharing_checks.insert("other".into(),Instant::now()-Duration::from_secs(6));assert!(matches!(c.upload_decision("other"),Decision::Deny(_)));c.upload_policy.allow_unknown=true;assert!(matches!(c.upload_decision("other"),Decision::Allow));
 let _=std::mem::size_of::<QueuedUpload>();
}
}

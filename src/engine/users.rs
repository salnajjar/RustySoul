use super::*;
use std::collections::BTreeSet;

impl Engine {
    pub(super) fn clear_presence(&mut self) {
        self.privileges.disconnected();
        if self.idle.reset(Instant::now()) {
            self.view.away = false;
        }
        self.room_controller
            .disconnected(&mut self.view.room_management);
        self.discovery.finish(
            &mut self.view.discovery,
            "Disconnected from Soulseek. Reconnect and refresh to try again.",
        );
        self.watched.clear();
        self.profiles.clear();
        self.view.room_members.clear();
        for user in self.view.users.values_mut() {
            user.status = "unknown".into();
            user.pending = false;
            user.profile_pending = false;
            if user.interests_pending {
                user.interests_error = Some(
                    "Disconnected before interests arrived. Refresh details after reconnecting."
                        .into(),
                );
            }
            user.interests_pending = false;
        }
    }

    pub(super) fn refresh_users(&mut self) {
        let own = self.secrets.username.clone();
        if !own.is_empty() {
            self.view.users.insert(
                own.clone(),
                UserView {
                    status: if self.client.is_none() {
                        "offline"
                    } else if self.view.away {
                        "away"
                    } else {
                        "online"
                    }
                    .into(),
                    exists: Some(true),
                    shared_files: Some(self.index.file_count()),
                    shared_folders: Some(self.index.folder_count()),
                    average_speed: self
                        .upload_speed
                        .reported(&self.profile, self.network.upload_limit_kib),
                    queue_size: Some(
                        self.client
                            .as_ref()
                            .map_or(0, |c| c.own_upload_availability().0),
                    ),
                    description: Some(self.profile.description.clone()),
                    interests: crate::discovery::Interests {
                        likes: self.profile.likes.clone(),
                        dislikes: self.profile.dislikes.clone(),
                        updated_at: 0,
                    },
                    has_picture: self.profile.picture.is_some(),
                    picture_revision: self.view.profile.revision.to_string(),
                    upload_slots: Some(self.network.upload_slots as u32),
                    slots_free: Some(
                        self.client
                            .as_ref()
                            .is_some_and(|c| c.own_upload_availability().1),
                    ),
                    ..Default::default()
                },
            );
        }
        let Some(c) = &self.client else {
            return;
        };
        self.view.room_members = self
            .view
            .joined_rooms
            .iter()
            .map(|room| {
                (
                    room.clone(),
                    c.room_members(room).into_iter().take(2000).collect(),
                )
            })
            .collect();
        // Bound subscriptions and send at most eight new requests each control tick.
        // Explicit lookups and buddies take priority over older search results.
        let mut wanted = c
            .uploads()
            .into_iter()
            .filter(|u| matches!(u.status, soulseek_rs::UploadStatus::InProgress))
            .map(|u| u.username)
            .collect::<Vec<_>>();
        let mut lookups: Vec<_> = self.lookups.iter().collect();
        lookups.sort_by_key(|(_, at)| std::cmp::Reverse(**at));
        wanted.extend(lookups.into_iter().map(|(u, _)| u.clone()));
        wanted.extend(self.view.buddies.clone());
        wanted.extend(
            self.view
                .discovery
                .people
                .iter()
                .map(|p| p.username.clone()),
        );
        self.focused_users
            .retain(|_, at| at.elapsed() < Duration::from_secs(1800));
        wanted.extend(self.focused_users.keys().cloned());
        wanted.extend(
            self.view
                .downloads
                .iter()
                .chain(&self.view.uploads)
                .map(|t| t.username.clone()),
        );
        wanted.extend(self.view.messages.iter().rev().flat_map(|m| {
            if m.private {
                vec![m.username.clone(), m.target.clone()]
            } else {
                vec![m.username.clone()]
            }
        }));
        wanted.extend(self.view.browse.keys().cloned());
        wanted.extend(self.view.room_members.values().flatten().cloned());
        wanted.extend(
            self.view
                .searches
                .iter()
                .rev()
                .flat_map(|s| s.results.iter().map(|f| f.username.clone())),
        );
        let mut seen = BTreeSet::new();
        wanted.retain(|u| {
            *u != own && crate::validation::username(u).is_ok() && seen.insert(u.clone())
        });
        wanted.truncate(512);
        let wanted_set: BTreeSet<_> = wanted.iter().cloned().collect();
        let removed: Vec<_> = self
            .watched
            .keys()
            .filter(|u| !wanted_set.contains(*u))
            .cloned()
            .collect();
        for u in removed {
            let _ = c.unwatch_user(&u);
            self.watched.remove(&u);
            self.view.users.remove(&u);
        }
        let mut sent = 0;
        for username in &wanted {
            if !self.watched.contains_key(username) && sent < 8 && c.watch_user(username).is_ok() {
                self.watched.insert(username.clone(), Instant::now());
                sent += 1;
                self.view.users.insert(
                    username.clone(),
                    UserView {
                        status: "unknown".into(),
                        pending: true,
                        ..Default::default()
                    },
                );
            }
            let user = self
                .view
                .users
                .entry(username.clone())
                .or_insert_with(|| UserView {
                    status: "unknown".into(),
                    ..Default::default()
                });
            if let Some(info) = c.user_info(username) {
                user.exists = info.exists;
                if info.exists == Some(false) {
                    user.status = "not_found".into();
                    user.pending = false;
                    user.error = Some("No Soulseek account exists with this exact username".into());
                }
                if let Some(p) = info.presence {
                    user.status = p.status.to_string();
                    user.pending = false;
                    user.error = None;
                }
                if let Some(s) = info.stats {
                    user.average_speed = Some(s.average_speed);
                    user.shared_files = Some(s.shared_files);
                    user.shared_folders = Some(s.shared_folders);
                }
            }
            if user.pending
                && self
                    .watched
                    .get(username)
                    .is_some_and(|at| at.elapsed() > Duration::from_secs(15))
            {
                user.pending = false;
                user.error = Some("User lookup timed out. Try refreshing while connected.".into());
            }
            if self.lookups.contains_key(username)
                && !user.pending
                && !self.profiles.contains_key(username)
            {
                if matches!(user.status.as_str(), "online" | "away") {
                    match c.request_peer_info(username) {
                        Ok(()) => {
                            user.profile_pending = true;
                            self.profiles.insert(username.clone(), Instant::now());
                        }
                        Err(e) => user.profile_error = Some(e.to_string()),
                    }
                } else {
                    user.profile_error = Some(
                        "Peer details are unavailable while this user is offline or unknown."
                            .into(),
                    );
                }
            }
            if user.profile_pending {
                if let Some(p) = c.peer_info(username) {
                    user.description = Some(p.description.chars().take(8192).collect());
                    user.profile_error = None;
                    match crate::profile::cache_picture(
                        &self.store.connection,
                        username,
                        p.picture.as_deref().map(Vec::as_slice),
                    ) {
                        Ok(revision) => {
                            user.has_picture = revision.is_some();
                            user.picture_revision = revision.unwrap_or_default();
                        }
                        Err(_) => {
                            user.has_picture = false;
                            user.picture_revision.clear();
                            user.profile_error = Some(
                                "This peer supplied an unsupported or invalid profile picture."
                                    .into(),
                            );
                        }
                    }
                    user.upload_slots = Some(p.total_uploads);
                    user.queue_size = Some(p.queue_size);
                    user.slots_free = Some(p.slots_free);
                    user.profile_pending = false;
                } else if self
                    .profiles
                    .get(username)
                    .is_some_and(|at| at.elapsed() > Duration::from_secs(45))
                {
                    user.profile_pending = false;
                    user.profile_error = Some(
                        "Peer profile timed out; server status and statistics are still shown."
                            .into(),
                    );
                }
            }
            if user.interests_pending {
                if let Some(interests) = c.user_interests(username) {
                    match crate::discovery::save_interests(&self.store.connection, interests, now())
                    {
                        Ok(interests) => {
                            user.interests = interests;
                            user.interests_error = None;
                        }
                        Err(e) => {
                            user.interests_error = Some(format!("Save user interests: {e:#}"))
                        }
                    }
                    user.interests_pending = false;
                } else if self
                    .lookups
                    .get(username)
                    .is_some_and(|at| at.elapsed() >= Duration::from_secs(20))
                {
                    user.interests_pending = false;
                    user.interests_error = Some("The server did not return this user's interests. Refresh details to try again.".into());
                }
            }
        }
        self.view
            .users
            .retain(|u, _| *u == own || wanted_set.contains(u));
    }
}

pub(super) fn rules(db: &rusqlite::Connection) -> Result<Vec<UserRule>> {
    Ok(db.prepare("SELECT username,ignored,banned FROM user_rules ORDER BY username COLLATE NOCASE,username")?.query_map([],|r|Ok(UserRule{username:r.get(0)?,ignored:r.get(1)?,banned:r.get(2)?}))?.collect::<rusqlite::Result<_>>()?)
}

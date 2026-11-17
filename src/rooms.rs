//! Room access and boards are server-confirmed. Cached roles never authorize
//! administration after reconnect. Large rosters stay in SQLite, not SSE.
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use soulseek_rs::{Client, RoomEvent};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct View {
    pub accept_invitations: bool,
    pub invitations_confirmed: Option<bool>,
    pub invitations_error: Option<String>,
    pub rooms: BTreeMap<String, Summary>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Summary {
    pub private: bool,
    pub users: Option<u32>,
    pub owner: Option<String>,
    pub member: bool,
    pub operator: bool,
    pub confirmed: bool,
    pub owner_live: bool,
    pub members_known: bool,
    pub operators_known: bool,
    pub board_known: bool,
    pub members: usize,
    pub operators: usize,
    pub board: usize,
    pub own_ticker: String,
    pub own_ticker_set: bool,
    pub updated_at: u64,
    pub revision: u64,
    pub pending: Option<String>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    AddMember,
    RemoveMember,
    AddOperator,
    RemoveOperator,
    LeaveMembership,
    ResignOperator,
    RelinquishOwnership,
}
impl Action {
    pub fn validate(&self, username: &Option<String>) -> std::result::Result<(), String> {
        if matches!(
            self,
            Self::AddMember | Self::RemoveMember | Self::AddOperator | Self::RemoveOperator
        ) {
            crate::validation::username(username.as_deref().ok_or("Choose a username")?)
        } else if username.is_some() {
            Err("This action does not take a username".into())
        } else {
            Ok(())
        }
    }
}
pub fn ticker_valid(text: &str) -> std::result::Result<(), String> {
    if text.is_empty() {
        Ok(())
    } else {
        crate::validation::text(text, 512)
    }
}
pub fn invitations(db: &Connection, account: &str) -> Result<bool> {
    Ok(db
        .query_row(
            "SELECT accept_invitations FROM room_controls WHERE account=?1",
            [account],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false))
}
pub fn load(db: &Connection, account: &str) -> Result<View> {
    let mut rooms: BTreeMap<String, Summary> = db
        .prepare("SELECT room,data FROM room_control_cache WHERE account=?1 ORDER BY room")?
        .query_map([account], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .map(|row| {
            let (room, data) = row?;
            let mut summary: Summary = serde_json::from_str(&data)?;
            summary.confirmed = false;
            summary.owner_live = false;
            summary.members_known = false;
            summary.operators_known = false;
            summary.board_known = false;
            summary.pending = None;
            summary.error = None;
            Ok((room, summary))
        })
        .collect::<Result<_>>()?;
    for row in db
        .prepare("SELECT room,ticker FROM room_preferences WHERE account=?1")?
        .query_map([account], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
    {
        let (room, ticker) = row?;
        let summary = rooms.entry(room).or_default();
        summary.own_ticker = ticker;
        summary.own_ticker_set = true;
    }
    for room in private_subscriptions(db, account)? {
        rooms.entry(room).or_default().private = true;
    }
    Ok(View {
        accept_invitations: invitations(db, account)?,
        rooms,
        ..Default::default()
    })
}
pub fn private_subscriptions(db: &Connection, account: &str) -> Result<Vec<String>> {
    Ok(db
        .prepare("SELECT room FROM private_room_subscriptions WHERE account=?1 ORDER BY rowid")?
        .query_map([account], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}
pub fn subscribe(db: &Connection, account: &str, room: &str, private: bool) -> Result<()> {
    let tx = crate::store::write_transaction(db)?;
    if private {
        tx.execute("DELETE FROM room_subscriptions WHERE room=?1", [room])?;
        tx.execute(
            "INSERT OR IGNORE INTO private_room_subscriptions VALUES(?1,?2)",
            params![account, room],
        )?;
    } else {
        tx.execute(
            "INSERT OR IGNORE INTO room_subscriptions VALUES(?1)",
            [room],
        )?;
    }
    tx.commit()?;
    Ok(())
}
pub fn unsubscribe(db: &Connection, account: &str, room: &str) -> Result<()> {
    let tx = crate::store::write_transaction(db)?;
    tx.execute("DELETE FROM room_subscriptions WHERE room=?1", [room])?;
    tx.execute(
        "DELETE FROM private_room_subscriptions WHERE account=?1 AND room=?2",
        params![account, room],
    )?;
    tx.commit()?;
    Ok(())
}
fn save(db: &Connection, account: &str, room: &str, s: &Summary) -> Result<()> {
    db.execute("INSERT INTO room_control_cache(account,room,data,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(account,room) DO UPDATE SET data=excluded.data,updated_at=excluded.updated_at",params![account,room,serde_json::to_string(s)?,s.updated_at.min(i64::MAX as u64) as i64])?;
    Ok(())
}
fn entry_count(db: &Connection, account: &str, room: &str, kind: &str) -> Result<usize> {
    Ok(db.query_row(
        "SELECT COUNT(*) FROM room_control_entries WHERE account=?1 AND room=?2 AND kind=?3",
        params![account, room, kind],
        |r| r.get::<_, i64>(0),
    )? as usize)
}
fn contains(db: &Connection, account: &str, room: &str, kind: &str, user: &str) -> Result<bool> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM room_control_entries WHERE account=?1 AND room=?2 AND kind=?3 AND username=?4 COLLATE NOCASE)",params![account,room,kind,user],|r|r.get(0))?)
}
fn replace(
    db: &Connection,
    account: &str,
    room: &str,
    kind: &str,
    values: Vec<(String, String)>,
    now: u64,
) -> Result<bool> {
    let limit = if kind == "board" { 2000 } else { 20_000 };
    let complete = values.len() <= limit;
    let tx = crate::store::write_transaction(db)?;
    tx.execute(
        "DELETE FROM room_control_entries WHERE account=?1 AND room=?2 AND kind=?3",
        params![account, room, kind],
    )?;
    for (user, text) in values.into_iter().take(limit).filter(|(u, t)| {
        crate::validation::username(u).is_ok() && (kind != "board" || ticker_valid(t).is_ok())
    }) {
        tx.execute(
            "INSERT OR REPLACE INTO room_control_entries VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                account,
                room,
                kind,
                user,
                text,
                now.min(i64::MAX as u64) as i64
            ],
        )?;
    }
    tx.commit()?;
    Ok(complete)
}
fn set_entry(
    db: &Connection,
    account: &str,
    room: &str,
    entry: (&str, &str, &str),
    added: bool,
    now: u64,
) -> Result<()> {
    let (kind, user, text) = entry;
    if crate::validation::username(user).is_err() {
        return Ok(());
    }
    if added {
        if !contains(db, account, room, kind, user)?
            && entry_count(db, account, room, kind)? >= if kind == "board" { 2000 } else { 20_000 }
        {
            return Ok(());
        }
        db.execute("INSERT INTO room_control_entries VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(account,room,kind,username) DO UPDATE SET text=excluded.text,updated_at=excluded.updated_at",params![account,room,kind,user,text,now.min(i64::MAX as u64) as i64])?;
    } else {
        db.execute("DELETE FROM room_control_entries WHERE account=?1 AND room=?2 AND kind=?3 AND username=?4 COLLATE NOCASE",params![account,room,kind,user])?;
    }
    Ok(())
}

#[derive(Default)]
pub struct Controller {
    pending: BTreeMap<String, (Instant, Expected)>,
    expired: BTreeMap<String, (Instant, Expected)>,
    invitation_at: Option<Instant>,
}
pub struct Session<'a> {
    pub db: &'a Connection,
    pub account: &'a str,
    pub client: Option<&'a Client>,
    pub joined: bool,
}
enum Expected {
    Action(Action, Option<String>),
    Join,
    Ticker(String),
}
impl Controller {
    pub fn pending(&self, room: &str) -> bool {
        self.pending.contains_key(room)
    }
    pub fn joining_private(&mut self, view: &mut View, room: &str) {
        self.expect(view, room, Expected::Join, "Joining private room…");
    }
    pub fn expects_private(&self, room: &str) -> bool {
        self.pending
            .get(room)
            .is_some_and(|(_, e)| matches!(e, Expected::Join))
    }
    pub fn disconnected(&mut self, view: &mut View) {
        self.pending.clear();
        self.expired.clear();
        self.invitation_at = None;
        view.invitations_confirmed = None;
        for s in view.rooms.values_mut() {
            s.confirmed = false;
            s.owner_live = false;
            s.members_known = false;
            s.operators_known = false;
            s.board_known = false;
            if s.pending.take().is_some() {
                s.error = Some("Disconnected before the server confirmed this request.".into());
            }
        }
    }
    pub fn connected(&mut self, view: &mut View) {
        view.invitations_confirmed = None;
        view.invitations_error = None;
        self.invitation_at = Some(Instant::now());
        for s in view.rooms.values_mut() {
            s.error = None;
        }
    }
    pub fn tick(&mut self, view: &mut View) {
        self.expired
            .retain(|_, (at, _)| at.elapsed() < Duration::from_secs(120));
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, (at, _))| at.elapsed() >= Duration::from_secs(20))
            .map(|(r, _)| r.clone())
            .collect();
        for room in expired {
            if let Some((_, expected)) = self.pending.remove(&room) {
                self.expired
                    .insert(room.clone(), (Instant::now(), expected));
            }
            if let Some(s) = view.rooms.get_mut(&room) {
                s.pending = None;
                s.error=Some("The server has not confirmed this request. Check the current room access or leave and rejoin before retrying.".into());
            }
        }
        if self
            .invitation_at
            .is_some_and(|at| at.elapsed() >= Duration::from_secs(20))
        {
            self.invitation_at = None;
            view.invitations_error=Some("Preference saved, but the server has not confirmed it. It will be sent again on connection.".into());
        }
    }
    fn expect(&mut self, view: &mut View, room: &str, expected: Expected, label: &str) {
        self.expired.remove(room);
        self.pending.insert(room.into(), (Instant::now(), expected));
        let s = view.rooms.entry(room.into()).or_default();
        s.pending = Some(label.into());
        s.error = None;
    }
    pub fn set_invitations(
        &mut self,
        db: &Connection,
        account: &str,
        client: Option<&Client>,
        view: &mut View,
        enabled: bool,
    ) -> Result<()> {
        db.execute("INSERT INTO room_controls VALUES(?1,?2) ON CONFLICT(account) DO UPDATE SET accept_invitations=excluded.accept_invitations",params![account,enabled])?;
        view.accept_invitations = enabled;
        view.invitations_confirmed = None;
        view.invitations_error = None;
        if let Some(c) = client {
            c.set_room_invitations_enabled(enabled)?;
            self.invitation_at = Some(Instant::now());
        }
        Ok(())
    }
    pub fn create(
        &mut self,
        db: &Connection,
        account: &str,
        c: &Client,
        view: &mut View,
        room: &str,
    ) -> Result<()> {
        ensure!(
            !self.pending.contains_key(room),
            "Wait for the current room request to finish"
        );
        c.join_private_room(room)?;
        subscribe(db, account, room, true)?;
        view.rooms.entry(room.into()).or_default().private = true;
        self.expect(
            view,
            room,
            Expected::Join,
            "Creating or joining private room…",
        );
        Ok(())
    }
    pub fn set_ticker(
        &mut self,
        session: Session<'_>,
        view: &mut View,
        room: &str,
        text: String,
    ) -> Result<()> {
        let Session {
            db,
            account,
            client: c,
            joined,
        } = session;
        ensure!(
            !self.pending.contains_key(room),
            "Wait for the current room request to finish"
        );
        db.execute("INSERT INTO room_preferences VALUES(?1,?2,?3) ON CONFLICT(account,room) DO UPDATE SET ticker=excluded.ticker",params![account,room,text])?;
        let s = view.rooms.entry(room.into()).or_default();
        s.own_ticker = text.clone();
        s.own_ticker_set = true;
        s.error = None;
        if joined && let Some(c) = c {
            let published:Option<String>=db.query_row("SELECT text FROM room_control_entries WHERE account=?1 AND room=?2 AND kind='board' AND username=?3 COLLATE NOCASE",params![account,room,account],|r|r.get(0)).optional()?;
            if s.board_known && published.as_deref().unwrap_or("") == text {
                return Ok(());
            }
            c.set_room_ticker(room, &text)?;
            self.expect(
                view,
                room,
                Expected::Ticker(text),
                "Publishing room-board message…",
            );
        }
        Ok(())
    }
    pub fn action(
        &mut self,
        session: Session<'_>,
        view: &mut View,
        room: &str,
        action: Action,
        username: Option<String>,
    ) -> Result<()> {
        let Session {
            db,
            account,
            client,
            joined,
        } = session;
        let c = client.context("Connect to Soulseek first")?;
        ensure!(
            !self.pending.contains_key(room),
            "Wait for the current room request to finish"
        );
        let s = view
            .rooms
            .get(room)
            .context("No private-room information is available")?;
        authorize(db, account, room, s, &action, username.as_deref(), joined)?;
        match action {
            Action::AddMember => c.add_room_member(room, username.as_deref().unwrap())?,
            Action::RemoveMember => c.remove_room_member(room, username.as_deref().unwrap())?,
            Action::AddOperator => c.add_room_operator(room, username.as_deref().unwrap())?,
            Action::RemoveOperator => c.remove_room_operator(room, username.as_deref().unwrap())?,
            Action::LeaveMembership => c.leave_private_room(room)?,
            Action::ResignOperator => c.resign_room_operatorship(room)?,
            Action::RelinquishOwnership => {
                c.disband_private_room(room)?;
                c.request_room_list()?;
            }
        }
        self.expect(
            view,
            room,
            Expected::Action(action, username),
            "Waiting for room access confirmation…",
        );
        Ok(())
    }

    /// Apply only validated server events. Returns private rooms eligible for
    /// automatic rejoin after the server has confirmed current membership.
    pub fn event(
        &mut self,
        db: &Connection,
        account: &str,
        view: &mut View,
        event: &RoomEvent,
        now: u64,
    ) -> Result<Vec<String>> {
        if let RoomEvent::InvitationsEnabled(enabled) = event {
            view.invitations_confirmed = Some(*enabled);
            view.invitations_error = if *enabled == view.accept_invitations {
                None
            } else {
                Some("The server reported a different invitation preference. Save it again to retry.".into())
            };
            self.invitation_at = None;
            return Ok(vec![]);
        }
        if let RoomEvent::PrivateList {
            owned,
            member,
            operated,
        } = event
        {
            let mut access: BTreeMap<String, (bool, bool)> = BTreeMap::new();
            for r in owned
                .iter()
                .take(256)
                .filter(|r| crate::validation::room(&r.name).is_ok())
            {
                access.insert(r.name.clone(), (true, true));
            }
            for r in member
                .iter()
                .take(256)
                .filter(|r| crate::validation::room(&r.name).is_ok())
            {
                access.entry(r.name.clone()).or_insert((false, true));
            }
            for r in operated
                .iter()
                .take(256)
                .filter(|r| crate::validation::room(r).is_ok())
            {
                access.entry(r.clone()).or_insert((false, true));
            }
            let existing: Vec<_> = view
                .rooms
                .iter()
                .filter(|(_, s)| s.private)
                .map(|(r, _)| r.clone())
                .collect();
            for room in existing {
                if self.expects_private(&room) {
                    continue;
                }
                if !access.contains_key(&room) {
                    let s = view.rooms.get_mut(&room).unwrap();
                    if owned.len() > 256 || member.len() > 256 || operated.len() > 256 {
                        s.confirmed = false;
                        s.owner_live = false;
                        s.error=Some("The server sent more private rooms than the current cache can hold. Existing subscription intent was retained.".into());
                        continue;
                    }
                    s.owner = None;
                    s.member = false;
                    s.operator = false;
                    s.confirmed = true;
                    s.owner_live = false;
                    db.execute(
                        "DELETE FROM room_control_entries WHERE account=?1 AND room=?2",
                        params![account, room],
                    )?;
                    unsubscribe(db, account, &room)?;
                    s.members = 0;
                    s.operators = 0;
                    s.board = 0;
                    save(db, account, &room, s)?;
                }
            }
            for (room, (owner, has_membership)) in &access {
                let s = view.rooms.entry(room.clone()).or_default();
                s.users = owned
                    .iter()
                    .chain(member.iter())
                    .find(|r| r.name == *room)
                    .map(|r| r.user_count);
                s.private = true;
                s.member = *has_membership;
                s.operator = operated.contains(room);
                s.confirmed = true;
                s.updated_at = now;
                if *owner {
                    s.owner = Some(account.into());
                } else if s
                    .owner
                    .as_deref()
                    .is_some_and(|o| o.eq_ignore_ascii_case(account))
                {
                    s.owner = None;
                    s.owner_live = false;
                }
                save(db, account, room, s)?;
            }
            let completed: Vec<_> = self
                .pending
                .iter()
                .chain(self.expired.iter())
                .filter_map(|(r, (_, e))| match e {
                    Expected::Action(Action::RelinquishOwnership, _)
                        if !access.get(r).is_some_and(|(owner, _)| *owner) =>
                    {
                        Some(r.clone())
                    }
                    _ => None,
                })
                .collect();
            for room in completed {
                self.confirm(view, &room);
            }
            return Ok(private_subscriptions(db, account)?
                .into_iter()
                .filter(|r| access.contains_key(r))
                .collect());
        }
        let room = match event {
            RoomEvent::PrivateJoined { room, .. }
            | RoomEvent::Joined { room, .. }
            | RoomEvent::Left { room }
            | RoomEvent::PrivateMembers { room, .. }
            | RoomEvent::PrivateOperators { room, .. }
            | RoomEvent::PrivateRosterChanged { room, .. }
            | RoomEvent::OwnStandingChanged { room, .. }
            | RoomEvent::Tickers { room, .. }
            | RoomEvent::TickerAdded { room, .. }
            | RoomEvent::TickerRemoved { room, .. }
            | RoomEvent::CantCreate { room } => room,
            _ => return Ok(vec![]),
        };
        if crate::validation::room(room).is_err() {
            return Ok(vec![]);
        }
        if !view.rooms.contains_key(room) && view.rooms.len() >= 256 {
            return Ok(vec![]);
        }
        let s = view.rooms.entry(room.clone()).or_default();
        s.updated_at = now;
        s.revision = s.revision.wrapping_add(1);
        match event {
            RoomEvent::PrivateJoined {
                owner, operators, ..
            } => {
                s.private = true;
                s.member = true;
                s.confirmed = true;
                s.owner_live = true;
                s.owner = if crate::validation::username(owner).is_ok() {
                    Some(owner.clone())
                } else {
                    None
                };
                if !owner.is_empty() && crate::validation::username(owner).is_err() {
                    s.owner_live = false;
                    s.error = Some("The server supplied an invalid room owner.".into());
                }
                s.operator = operators.iter().any(|u| u.eq_ignore_ascii_case(account));
                s.operators_known = replace(
                    db,
                    account,
                    room,
                    "operator",
                    operators
                        .iter()
                        .cloned()
                        .map(|u| (u, String::new()))
                        .collect(),
                    now,
                )?;
                subscribe(db, account, room, true)?;
            }
            RoomEvent::Joined { .. } => {
                if self
                    .pending
                    .get(room)
                    .is_some_and(|(_, e)| matches!(e, Expected::Join))
                {
                    s.pending = None;
                    s.error=Some("This name is a public room. RustySoul left it; choose another private-room name.".into());
                    self.pending.remove(room);
                    unsubscribe(db, account, room)?;
                }
                s.confirmed = true;
                s.private = false;
                s.owner = None;
                s.operator = false;
                s.owner_live = false;
            }
            RoomEvent::Left { .. } => {
                s.owner_live = false;
                s.board_known = false;
            }
            RoomEvent::PrivateMembers { users, .. } => {
                s.private = true;
                s.members_known = replace(
                    db,
                    account,
                    room,
                    "member",
                    users.iter().cloned().map(|u| (u, String::new())).collect(),
                    now,
                )?;
            }
            RoomEvent::PrivateOperators { users, .. } => {
                s.private = true;
                s.operators_known = replace(
                    db,
                    account,
                    room,
                    "operator",
                    users.iter().cloned().map(|u| (u, String::new())).collect(),
                    now,
                )?;
                s.operator = users.iter().any(|u| u.eq_ignore_ascii_case(account));
            }
            RoomEvent::PrivateRosterChanged {
                username,
                members,
                added,
                ..
            } => {
                set_entry(
                    db,
                    account,
                    room,
                    (if *members { "member" } else { "operator" }, username, ""),
                    *added,
                    now,
                )?;
                if *members && !added {
                    set_entry(db, account, room, ("operator", username, ""), false, now)?;
                }
                if !members && username.eq_ignore_ascii_case(account) {
                    s.operator = *added;
                }
            }
            RoomEvent::OwnStandingChanged {
                members, granted, ..
            } => {
                set_entry(
                    db,
                    account,
                    room,
                    (if *members { "member" } else { "operator" }, account, ""),
                    *granted,
                    now,
                )?;
                s.private = true;
                s.confirmed = true;
                if *members {
                    s.member = *granted;
                    if !granted {
                        s.operator = false;
                        s.owner = None;
                        s.owner_live = false;
                        db.execute(
                            "DELETE FROM room_control_entries WHERE account=?1 AND room=?2",
                            params![account, room],
                        )?;
                        unsubscribe(db, account, room)?;
                    }
                } else {
                    s.operator = *granted;
                }
            }
            RoomEvent::Tickers { tickers, .. } => {
                s.board_known = replace(
                    db,
                    account,
                    room,
                    "board",
                    tickers
                        .iter()
                        .map(|t| (t.username.clone(), t.ticker.clone()))
                        .collect(),
                    now,
                )?;
            }
            RoomEvent::TickerAdded {
                username, ticker, ..
            } if ticker_valid(ticker).is_ok() => {
                set_entry(db, account, room, ("board", username, ticker), true, now)?
            }
            RoomEvent::TickerRemoved { username, .. } => {
                set_entry(db, account, room, ("board", username, ""), false, now)?
            }
            RoomEvent::CantCreate { .. } => {
                s.error = Some(
                    "The server refused this room. It may be private, taken or unavailable.".into(),
                );
                s.pending = None;
                self.pending.remove(room);
                unsubscribe(db, account, room)?;
            }
            _ => {}
        }
        s.members = entry_count(db, account, room, "member")?;
        s.operators = entry_count(db, account, room, "operator")?;
        s.board = entry_count(db, account, room, "board")?;
        save(db, account, room, s)?;
        let confirmed = self
            .pending
            .get(room)
            .or_else(|| self.expired.get(room))
            .is_some_and(|(_, expected)| match (expected, event) {
                (Expected::Join, RoomEvent::PrivateJoined { .. }) => true,
                (
                    Expected::Ticker(text),
                    RoomEvent::TickerAdded {
                        username, ticker, ..
                    },
                ) => username.eq_ignore_ascii_case(account) && text == ticker,
                (Expected::Ticker(text), RoomEvent::TickerRemoved { username, .. }) => {
                    username.eq_ignore_ascii_case(account) && text.is_empty()
                }
                (
                    Expected::Action(action, user),
                    RoomEvent::PrivateRosterChanged {
                        username,
                        members,
                        added,
                        ..
                    },
                ) => {
                    user.as_deref()
                        .is_some_and(|u| u.eq_ignore_ascii_case(username))
                        && matches!(
                            (action, members, added),
                            (Action::AddMember, true, true)
                                | (Action::RemoveMember, true, false)
                                | (Action::AddOperator, false, true)
                                | (Action::RemoveOperator, false, false)
                        )
                }
                (
                    Expected::Action(Action::LeaveMembership, _),
                    RoomEvent::OwnStandingChanged {
                        members: true,
                        granted: false,
                        ..
                    },
                )
                | (
                    Expected::Action(Action::ResignOperator, _),
                    RoomEvent::OwnStandingChanged {
                        members: false,
                        granted: false,
                        ..
                    },
                ) => true,
                _ => false,
            });
        if confirmed {
            self.confirm(view, room);
        }
        Ok(vec![])
    }
    fn confirm(&mut self, view: &mut View, room: &str) {
        self.pending.remove(room);
        self.expired.remove(room);
        if let Some(s) = view.rooms.get_mut(room) {
            s.pending = None;
            s.error = None;
        }
    }
}

fn authorize(
    db: &Connection,
    account: &str,
    room: &str,
    s: &Summary,
    action: &Action,
    user: Option<&str>,
    joined: bool,
) -> Result<()> {
    ensure!(
        s.private && s.confirmed,
        "Wait for the server to confirm your current private-room access"
    );
    let owner = s
        .owner
        .as_deref()
        .is_some_and(|u| u.eq_ignore_ascii_case(account));
    ensure!(s.member || owner, "Your membership is no longer valid");
    match action {
        Action::LeaveMembership => {
            ensure!(
                s.member && !owner,
                "The owner must relinquish ownership before cancelling membership"
            );
            return Ok(());
        }
        Action::ResignOperator => {
            ensure!(
                s.operator && !owner,
                "You are not an operator who can resign"
            );
            return Ok(());
        }
        Action::RelinquishOwnership => {
            ensure!(
                owner,
                "Only the current room owner can relinquish ownership"
            );
            return Ok(());
        }
        _ => {}
    }
    ensure!(
        joined && s.owner_live,
        "Join this private room before managing access"
    );
    ensure!(
        owner || s.operator,
        "Only the owner or an operator can manage room access"
    );
    let user = user.context("Choose a username")?;
    ensure!(
        !user.eq_ignore_ascii_case(account),
        "Use your own membership or role controls for this action"
    );
    ensure!(
        !s.owner
            .as_deref()
            .is_some_and(|u| u.eq_ignore_ascii_case(user)),
        "The room owner's access cannot be changed here"
    );
    match action {
        Action::RemoveMember if !owner => ensure!(
            s.operators_known && !contains(db, account, room, "operator", user)?,
            "Operators cannot remove other operators; wait for the current operator list"
        ),
        Action::AddOperator | Action::RemoveOperator => {
            ensure!(owner, "Only the owner can change operators");
            if *action == Action::AddOperator {
                ensure!(
                    s.members_known && contains(db, account, room, "member", user)?,
                    "Add this user as a member first, and wait for server confirmation"
                );
            }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PageQuery {
    pub room: String,
    pub kind: String,
    pub filter: String,
    pub offset: usize,
}
impl Default for PageQuery {
    fn default() -> Self {
        Self {
            room: String::new(),
            kind: "board".into(),
            filter: String::new(),
            offset: 0,
        }
    }
}
#[derive(Serialize)]
pub struct Entry {
    pub username: String,
    pub text: String,
    pub operator: bool,
}
#[derive(Serialize)]
pub struct Page {
    pub total: usize,
    pub offset: usize,
    pub entries: Vec<Entry>,
}
pub fn page(db: &Connection, account: &str, q: &PageQuery) -> Result<Page> {
    crate::validation::room(&q.room).map_err(anyhow::Error::msg)?;
    ensure!(
        ["member", "operator", "board"].contains(&q.kind.as_str()),
        "Choose member, operator or board"
    );
    ensure!(
        q.filter.len() <= 128 && q.offset <= 1_000_000,
        "Invalid room filter or page"
    );
    let filter = q.filter.to_lowercase();
    let total=db.query_row("SELECT COUNT(*) FROM room_control_entries WHERE account=?1 AND room=?2 AND kind=?3 AND instr(lower(username||char(10)||text),?4)>0",params![account,q.room,q.kind,filter],|r|r.get::<_,i64>(0))? as usize;
    let entries=db.prepare("SELECT e.username,e.text,EXISTS(SELECT 1 FROM room_control_entries op WHERE op.account=e.account AND op.room=e.room AND op.kind='operator' AND op.username=e.username COLLATE NOCASE) FROM room_control_entries e WHERE e.account=?1 AND e.room=?2 AND e.kind=?3 AND instr(lower(e.username||char(10)||e.text),?4)>0 ORDER BY e.username COLLATE NOCASE,e.username LIMIT 100 OFFSET ?5")?.query_map(params![account,q.room,q.kind,filter,q.offset as i64],|r|Ok(Entry{username:r.get(0)?,text:r.get(1)?,operator:r.get(2)?}))?.collect::<rusqlite::Result<_>>()?;
    Ok(Page {
        total,
        offset: q.offset,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> crate::store::Store {
        crate::store::Store::open(std::path::Path::new(":memory:")).unwrap()
    }
    fn private() -> Summary {
        Summary {
            private: true,
            owner: Some("owner".into()),
            member: true,
            operator: true,
            confirmed: true,
            owner_live: true,
            members_known: true,
            operators_known: true,
            ..Default::default()
        }
    }
    #[test]
    fn paged_rosters_pass_2500_without_entering_live_snapshots() {
        let store = store();
        let db = &store.connection;
        let mut c = Controller::default();
        let mut v = View::default();
        c.event(
            db,
            "owner",
            &mut v,
            &RoomEvent::PrivateMembers {
                room: "club".into(),
                users: (0..3001).map(|n| format!("user-{n:04}")).collect(),
            },
            1,
        )
        .unwrap();
        c.event(
            db,
            "owner",
            &mut v,
            &RoomEvent::PrivateOperators {
                room: "club".into(),
                users: vec!["user-3000".into()],
            },
            2,
        )
        .unwrap();
        let result = page(
            db,
            "owner",
            &PageQuery {
                room: "club".into(),
                kind: "member".into(),
                offset: 3000,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total, 3001);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].username, "user-3000");
        assert!(result.entries[0].operator);
        assert!(serde_json::to_string(&v).unwrap().len() < 1000);
        assert_eq!(
            page(
                db,
                "other-account",
                &PageQuery {
                    room: "club".into(),
                    kind: "member".into(),
                    ..Default::default()
                }
            )
            .unwrap()
            .total,
            0
        );
    }
    #[test]
    fn cached_owner_roles_never_authorize_after_restart() {
        let store = store();
        let db = &store.connection;
        save(db, "owner", "club", &private()).unwrap();
        let v = load(db, "owner").unwrap();
        let s = &v.rooms["club"];
        assert!(!s.confirmed && !s.owner_live);
        for action in [
            Action::AddMember,
            Action::RelinquishOwnership,
            Action::RemoveMember,
        ] {
            assert!(authorize(db, "owner", "club", s, &action, Some("bob"), true).is_err());
        }
    }
    #[test]
    fn operators_cannot_remove_owner_or_other_operators_or_promote_members() {
        let store = store();
        let db = &store.connection;
        let s = private();
        set_entry(db, "operator", "club", ("operator", "alice", ""), true, 1).unwrap();
        assert!(
            authorize(
                db,
                "operator",
                "club",
                &s,
                &Action::RemoveMember,
                Some("owner"),
                true
            )
            .is_err()
        );
        assert!(
            authorize(
                db,
                "operator",
                "club",
                &s,
                &Action::RemoveMember,
                Some("alice"),
                true
            )
            .is_err()
        );
        assert!(
            authorize(
                db,
                "operator",
                "club",
                &s,
                &Action::AddOperator,
                Some("bob"),
                true
            )
            .is_err()
        );
        assert!(
            authorize(
                db,
                "operator",
                "club",
                &s,
                &Action::RemoveMember,
                Some("bob"),
                true
            )
            .is_ok()
        );
        assert!(
            authorize(
                db,
                "operator",
                "club",
                &s,
                &Action::AddMember,
                Some("bob"),
                false
            )
            .is_err()
        );
        assert!(
            authorize(
                db,
                "owner",
                "club",
                &s,
                &Action::AddOperator,
                Some("bob"),
                true
            )
            .is_err()
        );
        set_entry(db, "owner", "club", ("member", "bob", ""), true, 1).unwrap();
        assert!(
            authorize(
                db,
                "owner",
                "club",
                &s,
                &Action::AddOperator,
                Some("bob"),
                true
            )
            .is_ok()
        );
        assert!(
            authorize(
                db,
                "owner",
                "club",
                &s,
                &Action::LeaveMembership,
                None,
                true
            )
            .is_err()
        );
    }
    #[test]
    fn revocation_discards_access_data_and_private_autojoin_without_touching_other_accounts() {
        let store = store();
        let db = &store.connection;
        let mut c = Controller::default();
        let mut v = View::default();
        v.rooms.insert("club".into(), private());
        subscribe(db, "owner", "club", true).unwrap();
        subscribe(db, "other", "club", true).unwrap();
        set_entry(db, "owner", "club", ("board", "alice", "hello"), true, 1).unwrap();
        c.event(
            db,
            "owner",
            &mut v,
            &RoomEvent::OwnStandingChanged {
                room: "club".into(),
                members: true,
                granted: false,
            },
            2,
        )
        .unwrap();
        assert!(!v.rooms["club"].member && !v.rooms["club"].operator);
        assert!(private_subscriptions(db, "owner").unwrap().is_empty());
        assert_eq!(private_subscriptions(db, "other").unwrap(), vec!["club"]);
        assert_eq!(entry_count(db, "owner", "club", "board").unwrap(), 0);
    }
    #[test]
    fn requests_wait_for_matching_confirmation_and_expire_without_optimistic_role_changes() {
        let store = store();
        let db = &store.connection;
        let mut c = Controller::default();
        let mut v = View::default();
        c.expect(
            &mut v,
            "club",
            Expected::Action(Action::AddMember, Some("bob".into())),
            "Waiting",
        );
        c.event(
            db,
            "owner",
            &mut v,
            &RoomEvent::PrivateRosterChanged {
                room: "club".into(),
                username: "alice".into(),
                members: true,
                added: true,
            },
            1,
        )
        .unwrap();
        assert!(v.rooms["club"].pending.is_some());
        c.event(
            db,
            "owner",
            &mut v,
            &RoomEvent::PrivateRosterChanged {
                room: "club".into(),
                username: "bob".into(),
                members: true,
                added: true,
            },
            2,
        )
        .unwrap();
        assert!(v.rooms["club"].pending.is_none());
        c.expect(
            &mut v,
            "club",
            Expected::Ticker("new message".into()),
            "Publishing",
        );
        c.pending.get_mut("club").unwrap().0 = Instant::now() - Duration::from_secs(21);
        c.tick(&mut v);
        assert!(v.rooms["club"].pending.is_none());
        assert!(
            v.rooms["club"]
                .error
                .as_deref()
                .unwrap()
                .contains("not confirmed")
        );
        c.event(
            db,
            "owner",
            &mut v,
            &RoomEvent::TickerAdded {
                room: "club".into(),
                username: "owner".into(),
                ticker: "new message".into(),
            },
            3,
        )
        .unwrap();
        assert!(
            v.rooms["club"].error.is_none(),
            "a late matching confirmation clears the obsolete timeout"
        );
        c.connected(&mut v);
        c.event(db, "owner", &mut v, &RoomEvent::InvitationsEnabled(true), 3)
            .unwrap();
        assert!(!v.accept_invitations);
        assert!(v.invitations_error.is_some());
    }
    #[test]
    fn private_subscriptions_wait_for_live_access_and_pending_creation_survives_an_older_list() {
        let store = store();
        let db = &store.connection;
        let mut c = Controller::default();
        let mut v = View::default();
        subscribe(db, "owner", "club", true).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM room_subscriptions", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        c.expect(&mut v, "club", Expected::Join, "Creating");
        v.rooms.get_mut("club").unwrap().private = true;
        assert!(
            c.event(
                db,
                "owner",
                &mut v,
                &RoomEvent::PrivateList {
                    owned: vec![],
                    member: vec![],
                    operated: vec![]
                },
                1
            )
            .unwrap()
            .is_empty()
        );
        assert_eq!(private_subscriptions(db, "owner").unwrap(), vec!["club"]);
        c.event(
            db,
            "owner",
            &mut v,
            &RoomEvent::PrivateJoined {
                room: "club".into(),
                users: vec![],
                owner: "owner".into(),
                operators: vec![],
            },
            2,
        )
        .unwrap();
        assert!(v.rooms["club"].pending.is_none());
        assert!(v.rooms["club"].confirmed);
        let rejoin = c
            .event(
                db,
                "owner",
                &mut v,
                &RoomEvent::PrivateList {
                    owned: vec![soulseek_rs::RoomInfo {
                        name: "club".into(),
                        user_count: 1,
                    }],
                    member: vec![],
                    operated: vec![],
                },
                3,
            )
            .unwrap();
        assert_eq!(rejoin, vec!["club"]);
    }
    #[test]
    fn board_preferences_and_literal_paged_text_survive_restart_and_clear() {
        let store = store();
        let db = &store.connection;
        let mut c = Controller::default();
        let mut v = View::default();
        c.set_ticker(
            Session {
                db,
                account: "owner",
                client: None,
                joined: false,
            },
            &mut v,
            "club",
            "<script>literal</script>".into(),
        )
        .unwrap();
        c.event(
            db,
            "owner",
            &mut v,
            &RoomEvent::TickerAdded {
                room: "club".into(),
                username: "alice".into(),
                ticker: "<img src=x>".into(),
            },
            1,
        )
        .unwrap();
        let restored = load(db, "owner").unwrap();
        assert_eq!(
            restored.rooms["club"].own_ticker,
            "<script>literal</script>"
        );
        assert!(restored.rooms["club"].own_ticker_set);
        assert!(!restored.rooms["club"].board_known);
        assert_eq!(
            page(
                db,
                "owner",
                &PageQuery {
                    room: "club".into(),
                    filter: "<img".into(),
                    ..Default::default()
                }
            )
            .unwrap()
            .entries[0]
                .text,
            "<img src=x>"
        );
        c.set_ticker(
            Session {
                db,
                account: "owner",
                client: None,
                joined: false,
            },
            &mut v,
            "club",
            String::new(),
        )
        .unwrap();
        assert_eq!(load(db, "owner").unwrap().rooms["club"].own_ticker, "");
        c.event(
            db,
            "owner",
            &mut v,
            &RoomEvent::TickerRemoved {
                room: "club".into(),
                username: "alice".into(),
            },
            2,
        )
        .unwrap();
        assert_eq!(entry_count(db, "owner", "club", "board").unwrap(), 0);
    }
    #[test]
    fn room_commands_validate_targets_and_board_text() {
        for input in [
            r#"{"type":"private_room_action","room":"club","action":"add_member"}"#,
            r#"{"type":"private_room_action","room":"club","action":"leave_membership","username":"bob"}"#,
            r#"{"type":"set_room_ticker","room":"club","text":"bad\nmessage"}"#,
            r#"{"type":"create_private_room","room":"  invalid"}"#,
        ] {
            let command = serde_json::from_str::<crate::model::Command>(input);
            assert!(command.is_err() || command.unwrap().validate().is_err());
        }
        assert!(ticker_valid("").is_ok());
        assert!(ticker_valid(&"x".repeat(513)).is_err());
    }
}

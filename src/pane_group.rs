//! Grouping two sessions into one split content area, and
//! taking them back apart.
//!
//! The review: *"Implement split pane view, rightclick on sidebar or drag and
//! drop a sidebar session to the left, right, up, down of the content area so
//! the open terminal, that should add the other terminal window there and make
//! a group on the sidebar. Add some good UI to ungroup them."*
//!
//! # What already existed, and what did not
//!
//! Splitting was already shipped: [`crate::pane_layout::PaneLayout`] is a
//! tmux-style tree, `Split pane right` / `Split pane down` are bound, and the
//! framework's pane grid renders and resizes it. Every one of those splits
//! **spawns a new shell**. What did not exist is moving an *already running*
//! session — its live PTY, its scrollback, its cwd — into the layout beside
//! the one on screen, and being able to pull it back out again.
//!
//! # The ownership move, and why the PTY survives it
//!
//! A [`Pane`] is the thing that owns a terminal surface; the live
//! [`PtySession`](crate::pty::PtySession) it talks to lives in `App::ptys`,
//! keyed by **pane id**, in one process-wide map that is not per session. So
//! grouping is a pure move of `Pane` values between two `Session`s plus a
//! splice of the layout trees: no PTY is closed, reopened, resized away or
//! re-spawned, and no scrollback is copied. That is the whole reason this is
//! expressible at all, and it is why these functions take `&mut Vec<Session>`
//! and never touch `ptys`.
//!
//! # Ungrouping is not "close and reopen"
//!
//! A group records, per merged session, what that session *was*: its title,
//! its kind, its SSH host id, and exactly which panes it contributed
//! (`GroupMember`). Ungrouping replays that record — the panes move back
//! into a restored session with its own name in the sidebar. Without the
//! record, "ungroup" could only mean "close it", which loses a running shell.

use crate::model::{CommandHistory, Pane, Session, SessionKind, SessionStatus};
use crate::pane_layout::{PaneLayout, SplitDirection};

/// Which edge of the content area a session was dropped on, or which
/// direction the context menu asked for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GroupEdge {
    Left,
    Right,
    Up,
    Down,
}

impl GroupEdge {
    /// Every edge, in the order the context menu lists them.
    pub(crate) const ALL: [GroupEdge; 4] = [
        GroupEdge::Left,
        GroupEdge::Right,
        GroupEdge::Up,
        GroupEdge::Down,
    ];

    /// The split axis this edge implies.
    pub(crate) fn direction(self) -> SplitDirection {
        match self {
            GroupEdge::Left | GroupEdge::Right => SplitDirection::Horizontal,
            GroupEdge::Up | GroupEdge::Down => SplitDirection::Vertical,
        }
    }

    /// Whether the incoming session takes the **leading** side of the split.
    pub(crate) fn places_first(self) -> bool {
        matches!(self, GroupEdge::Left | GroupEdge::Up)
    }

    /// A stable key for menu entries and test lookups.
    pub(crate) fn key(self) -> &'static str {
        match self {
            GroupEdge::Left => "left",
            GroupEdge::Right => "right",
            GroupEdge::Up => "up",
            GroupEdge::Down => "down",
        }
    }

    /// The Lucide icon naming this edge. `panel-*-open` is the shipped family
    /// for "a surface arrives on this side", which is precisely the gesture.
    pub(crate) fn icon(self) -> &'static str {
        match self {
            GroupEdge::Left => "panel-left-open",
            GroupEdge::Right => "panel-right-open",
            GroupEdge::Up => "panel-top-open",
            GroupEdge::Down => "panel-bottom-open",
        }
    }

    /// The visible menu label.
    pub(crate) fn label(self) -> &'static str {
        match self {
            GroupEdge::Left => crate::i18n::label("Group to the left"),
            GroupEdge::Right => crate::i18n::label("Group to the right"),
            GroupEdge::Up => crate::i18n::label("Group above"),
            GroupEdge::Down => crate::i18n::label("Group below"),
        }
    }

    /// This edge's drop zone within `area` (a pane, or the whole content
    /// area), as the rectangle the drag host hit-tests.
    ///
    /// # One authority for the geometry
    ///
    /// These rectangles are what the app registers with
    /// [`enz_drag_item_surface`](enzui_widgets::drag_item_surface), so the
    /// shared drag host hit-tests against the real laid-out bounds, and they
    /// are what a unit test resolves: the zone a user aims at and the zone the
    /// host hits are one rectangle by construction.
    ///
    /// # The shape
    ///
    /// The four zones tile `area` with no gap and no overlap (the host takes
    /// the first registered item containing the pointer, so an overlap would
    /// make registration order the tie-break). Left and Right take the full
    /// height of an outer band [`SIDE_FRACTION`] wide; Up and Down split the
    /// centre column at half height. Every point of a pane therefore means a
    /// drop somewhere; a drag released outside the content area, or cancelled
    /// with Escape, changes nothing.
    pub(crate) fn zone_in(self, area: iced::Rectangle) -> iced::Rectangle {
        let w = area.width.max(0.0);
        let h = area.height.max(0.0);
        let side = w * SIDE_FRACTION;
        let (x, y, width, height) = match self {
            GroupEdge::Left => (0.0, 0.0, side, h),
            GroupEdge::Right => (w - side, 0.0, side, h),
            GroupEdge::Up => (side, 0.0, w - 2.0 * side, h / 2.0),
            GroupEdge::Down => (side, h / 2.0, w - 2.0 * side, h - h / 2.0),
        };
        iced::Rectangle {
            x: area.x + x,
            y: area.y + y,
            width: width.max(0.0),
            height: height.max(0.0),
        }
    }

    /// Where the dropped session would land in `area`: the half on this
    /// edge's side. The drag highlight paints this, not the hit zone, so the
    /// user sees the result before letting go.
    pub(crate) fn preview_in(self, area: iced::Rectangle) -> iced::Rectangle {
        let (w, h) = (area.width.max(0.0), area.height.max(0.0));
        let (x, y, width, height) = match self {
            GroupEdge::Left => (0.0, 0.0, w / 2.0, h),
            GroupEdge::Right => (w / 2.0, 0.0, w - w / 2.0, h),
            GroupEdge::Up => (0.0, 0.0, w, h / 2.0),
            GroupEdge::Down => (0.0, h / 2.0, w, h - h / 2.0),
        };
        let inset = PREVIEW_INSET.min(width / 4.0).min(height / 4.0);
        iced::Rectangle {
            x: area.x + x + inset,
            y: area.y + y + inset,
            width: (width - 2.0 * inset).max(0.0),
            height: (height - 2.0 * inset).max(0.0),
        }
    }

    /// The keyed drop-target id this edge registers under.
    ///
    /// Prefixed, so it can never collide with a session id in the same drag
    /// session — the two live in one host and are told apart by exactly this.
    ///
    /// `pane` names the pane the edge belongs to; `None` is an edge of the
    /// whole content area.
    pub(crate) fn item_id(self, pane: Option<&str>) -> enzui_core::drag::ItemId {
        enzui_core::drag::ItemId::new(match pane {
            Some(pane) => format!("{EDGE_ITEM_PREFIX}{}:{pane}", self.key()),
            None => format!("{EDGE_ITEM_PREFIX}{}", self.key()),
        })
    }

    /// The edge and pane an [`item_id`](Self::item_id) names, or `None` when
    /// the id is not an edge target at all (a session row, a stale id from
    /// another collection).
    pub(crate) fn from_item_id(
        id: &enzui_core::drag::ItemId,
    ) -> Option<(GroupEdge, Option<String>)> {
        let rest = id.0.strip_prefix(EDGE_ITEM_PREFIX)?;
        let (key, pane) = match rest.split_once(':') {
            Some((key, pane)) => (key, Some(pane.to_owned())),
            None => (rest, None),
        };
        let edge = GroupEdge::ALL.into_iter().find(|e| e.key() == key)?;
        Some((edge, pane))
    }
}

/// Id prefix for the four content-area edge drop targets.
pub(crate) const EDGE_ITEM_PREFIX: &str = "term-edge:";

/// The drag collection the sidebar's session rows register under, per section
///. Two sections means two collections: the host refuses a
/// duplicate collection id, and a shared one would make the second section's
/// rows unreachable as drag sources.
pub(crate) fn session_collection(section: &str) -> enzui_core::drag::CollectionId {
    enzui_core::drag::CollectionId(format!("term-sessions-{section}"))
}

/// The drag collection the terminal content area registers under.
pub(crate) const CONTENT_COLLECTION: &str = "term-content";

/// The share of a drop area's width each side band (Left, Right) takes.
pub(crate) const SIDE_FRACTION: f32 = 0.3;

/// How far a drop preview stands in from the area it previews, in logical
/// pixels, so it reads as a tile rather than a stain.
pub(crate) const PREVIEW_INSET: f32 = 6.0;

/// One session that was merged into another, and everything needed to give it
/// back.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GroupMember {
    /// The merged session's own id, reused when it is restored so anything
    /// keyed on it (the sidebar's selection key, a context menu target) lands
    /// on the same session it did before.
    pub session_id: String,
    /// Its title, shown on the group's ungroup control and restored verbatim.
    pub title: String,
    pub subtitle: String,
    pub icon: String,
    pub status: SessionStatus,
    pub kind: SessionKind,
    pub ssh_host_id: Option<String>,
    pub connection_error: Option<String>,
    pub pinned: bool,
    pub detached: bool,
    /// The panes it contributed, in its own layout order. Ungrouping moves
    /// exactly these back out.
    pub pane_ids: Vec<String>,
    /// The exact split tree, including directions and ratios, from before the
    /// move. Pane closes prune this tree together with `pane_ids`.
    pub layout: PaneLayout,
    /// Stable id of the pane that was active before grouping.
    pub active_pane_id: String,
    pub in_app_history: CommandHistory,
    /// Groups already nested in the arriving session. Moving a grouped source
    /// must not silently discard its own ungroup history.
    pub members: Vec<GroupMember>,
}

impl GroupMember {
    pub(crate) fn apply_shell_event(
        &mut self,
        pane: &str,
        event: &crate::shell_history::ShellEvent,
        panes: &[Pane],
    ) {
        if self.pane_ids.iter().any(|id| id == pane) {
            self.in_app_history.apply_shell_event(event);
            if self.connection_error.is_none() {
                self.status = if panes.iter().any(|pane| {
                    self.pane_ids.contains(&pane.id) && pane.status == SessionStatus::Running
                }) {
                    SessionStatus::Running
                } else {
                    SessionStatus::Idle
                };
            }
            for member in &mut self.members {
                member.apply_shell_event(pane, event, panes);
            }
        }
    }

    pub(crate) fn split_owned_pane(
        &mut self,
        source: &str,
        direction: SplitDirection,
        new_id: &str,
    ) {
        if self.pane_ids.iter().any(|id| id == source) {
            if self.layout.split_leaf(source, direction, new_id) {
                self.pane_ids.push(new_id.to_owned());
                self.active_pane_id = new_id.to_owned();
            }
            for member in &mut self.members {
                member.split_owned_pane(source, direction, new_id);
            }
        }
    }

    /// Remove a closed pane from this member and every nested member. Returns
    /// `false` when the member no longer owns any live pane and must be dropped.
    pub(crate) fn forget_closed_pane(&mut self, pane_id: &str) -> bool {
        self.members
            .retain_mut(|member| member.forget_closed_pane(pane_id));
        if self.pane_ids.iter().any(|id| id == pane_id) {
            self.pane_ids.retain(|id| id != pane_id);
            if self.pane_ids.is_empty() {
                return false;
            }
            let _ = self.layout.remove_leaf(pane_id);
            if self.active_pane_id == pane_id {
                self.active_pane_id = self.pane_ids[0].clone();
            }
        }
        true
    }
}

/// Why a grouping request was refused. Every variant is a state the UI can
/// reach, so each one is a message a user can be shown rather than a silent
/// no-op.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GroupError {
    /// Source and target are the same session.
    SameSession,
    /// One of the two ids is not a live session.
    NoSuchSession,
    /// The merged layout would exceed [`crate::pane_layout::MAX_PANES`].
    PaneLimit,
    /// The source is detached (its own window); merging it would make it
    /// disappear from a window the user is looking at.
    SourceDetached,
}

impl GroupError {
    /// The sentence shown in the pane feedback strip.
    pub(crate) fn message(self) -> &'static str {
        match self {
            GroupError::SameSession => "A session cannot be grouped with itself.",
            GroupError::NoSuchSession => "That session is no longer open.",
            GroupError::PaneLimit => {
                "Pane limit reached: at most 8 terminal surfaces can be rendered."
            }
            GroupError::SourceDetached => {
                "Re-dock the detached tab before grouping it into another session."
            }
        }
    }
}

/// [`group_into_pane`] around the target's whole layout.
#[cfg(test)]
pub(crate) fn group_into(
    sessions: &mut Vec<Session>,
    source_id: &str,
    target_id: &str,
    edge: GroupEdge,
) -> Result<usize, GroupError> {
    group_into_pane(sessions, source_id, target_id, edge, None)
}

/// Move every pane of `source_id` into `target_id`'s layout at `edge`,
/// removing the source session from the sidebar and recording it as a group
/// member of the target.
///
/// Returns the index `target_id` now occupies, so the caller can keep the
/// active selection on the session the user is looking at — removing the
/// source shifts every index after it, and an index captured before the call
/// would silently select a neighbour.
///
/// `pane` names the target pane the edge belongs to: that pane is split and
/// the rest of the layout kept. `None`, or a pane the target does not have,
/// splits around the whole layout.
pub(crate) fn group_into_pane(
    sessions: &mut Vec<Session>,
    source_id: &str,
    target_id: &str,
    edge: GroupEdge,
    pane: Option<&str>,
) -> Result<usize, GroupError> {
    if source_id == target_id {
        return Err(GroupError::SameSession);
    }
    let source_idx = sessions
        .iter()
        .position(|s| s.id == source_id)
        .ok_or(GroupError::NoSuchSession)?;
    let target_idx = sessions
        .iter()
        .position(|s| s.id == target_id)
        .ok_or(GroupError::NoSuchSession)?;
    if sessions[source_idx].detached {
        return Err(GroupError::SourceDetached);
    }
    let merged_panes = sessions[source_idx].panes.len() + sessions[target_idx].panes.len();
    if merged_panes > crate::pane_layout::MAX_PANES {
        return Err(GroupError::PaneLimit);
    }

    let source = sessions.remove(source_idx);
    // `remove` shifted everything after `source_idx` down by one.
    let target_idx = if target_idx > source_idx {
        target_idx - 1
    } else {
        target_idx
    };

    let source_active_pane_id = source
        .panes
        .get(source.active_pane)
        .map(|pane| pane.id.clone())
        .expect("a live session always has an active pane");
    let member = GroupMember {
        session_id: source.id.clone(),
        title: source.title.clone(),
        subtitle: source.subtitle.clone(),
        icon: source.icon.clone(),
        status: source.status,
        kind: source.kind,
        ssh_host_id: source.ssh_host_id.clone(),
        connection_error: source.connection_error.clone(),
        pinned: source.pinned,
        detached: source.detached,
        pane_ids: source.layout.leaves_in_order(),
        layout: source.layout.clone(),
        active_pane_id: source_active_pane_id.clone(),
        in_app_history: source.in_app_history.clone(),
        members: source.group_members,
    };
    let source_layout = source.layout;
    let source_panes = source.panes;

    let target = &mut sessions[target_idx];
    // Splice the incoming tree whole, at the pane it was dropped on or around
    // the whole layout. It may itself be a split tree, so this keeps an
    // already-tiled source intact instead of rebuilding it leaf by leaf.
    let at_pane = pane.is_some_and(|pane| {
        target.layout.split_leaf_with(
            pane,
            edge.direction(),
            source_layout.clone(),
            edge.places_first(),
        )
    });
    if !at_pane {
        target
            .layout
            .split_root(edge.direction(), source_layout, edge.places_first());
    }
    // The arriving panes are appended, and the pane that was active in the
    // source remains active after the move. Picking the first arrival loses
    // focus whenever the source was a non-default nested workspace.
    target.panes.extend(source_panes);
    target.active_pane = target
        .panes
        .iter()
        .position(|pane| pane.id == source_active_pane_id)
        .expect("the source active pane moved into the target");
    target.group_members.push(member);
    target
        .pane_grid
        .replace_tree(target.layout.pane_grid_tree());
    Ok(target_idx)
}

/// Take group member `member_index` back out of the session at `session_idx`,
/// restoring it as its own sidebar session immediately after the group
///.
///
/// Returns the index of the restored session, or `None` when the request does
/// not name a live group member.
pub(crate) fn ungroup(
    sessions: &mut Vec<Session>,
    session_idx: usize,
    member_index: usize,
) -> Option<usize> {
    let group = sessions.get_mut(session_idx)?;
    if member_index >= group.group_members.len() {
        return None;
    }
    let member = group.group_members.remove(member_index);
    let live_member_ids: Vec<String> = member
        .pane_ids
        .iter()
        .filter(|pane_id| group.panes.iter().any(|pane| &pane.id == *pane_id))
        .cloned()
        .collect();
    // Never make the target empty. In particular, if its last original pane
    // was somehow removed, retain the member record rather than moving every
    // remaining pane away while `remove_leaf` leaves a dangling root behind.
    if live_member_ids.is_empty() || live_member_ids.len() == group.panes.len() {
        group.group_members.insert(member_index, member);
        return None;
    }

    // Pull the member's panes out of the group's layout first: a pane whose
    // leaf is still in the tree but whose `Pane` has moved away renders as a
    // missing surface, so the preconditions above make the edit atomic.
    let mut restored: Vec<Pane> = Vec::new();
    for pane_id in &live_member_ids {
        let pos = group
            .panes
            .iter()
            .position(|p| &p.id == pane_id)
            .expect("live_member_ids was derived from these panes");
        restored.push(group.panes.remove(pos));
        assert!(
            group.layout.remove_leaf(pane_id),
            "a recorded member pane must remain in the group layout"
        );
    }
    group.active_pane = group.active_pane.min(group.panes.len().saturating_sub(1));
    group.pane_grid.replace_tree(group.layout.pane_grid_tree());

    let layout = member.layout;
    debug_assert_eq!(layout.leaves_in_order(), live_member_ids);
    let active_pane = restored
        .iter()
        .position(|pane| pane.id == member.active_pane_id)
        .unwrap_or(0);
    let pane_grid = enzui_widgets::EnzPaneGridState::new(layout.pane_grid_tree());
    let session = Session {
        id: member.session_id,
        title: member.title,
        subtitle: member.subtitle,
        icon: member.icon,
        status: member.status,
        kind: member.kind,
        ssh_host_id: member.ssh_host_id,
        connection_error: member.connection_error,
        pinned: member.pinned,
        detached: member.detached,
        panes: restored,
        active_pane,
        layout,
        pane_grid,
        in_app_history: member.in_app_history,
        group_members: member.members,
    };
    let at = session_idx + 1;
    sessions.insert(at, session);
    Some(at)
}

/// Extract only the selected pane from a multi-pane session into its own
/// sidebar session. This is the model half of "Detach pane": the caller opens
/// a window for the returned session id.
///
/// If the pane is the sole pane contributed by a grouped member, use the
/// ordinary ungroup route so that member's exact identity and metadata return.
/// Otherwise carve out only that leaf while preserving the remaining topology.
pub(crate) fn extract_pane_for_detach(
    sessions: &mut Vec<Session>,
    session_idx: usize,
) -> Option<usize> {
    let group = sessions.get(session_idx)?;
    if group.panes.len() <= 1 {
        return Some(session_idx);
    }
    let pane_id = group.panes.get(group.active_pane)?.id.clone();
    if let Some(member_index) = group
        .group_members
        .iter()
        .position(|member| member.pane_ids.len() == 1 && member.pane_ids[0] == pane_id)
    {
        return ungroup(sessions, session_idx, member_index);
    }

    let group = sessions.get_mut(session_idx)?;
    let pane_index = group.panes.iter().position(|pane| pane.id == pane_id)?;
    // Validate and edit the layout before moving the live pane, so a corrupted
    // model fails closed instead of losing the pane from storage.
    if !group.layout.remove_leaf(&pane_id) {
        return None;
    }
    let pane = group.panes.remove(pane_index);
    let owner = group
        .group_members
        .iter()
        .find(|member| member.pane_ids.iter().any(|id| id == &pane_id))
        .cloned();
    group
        .group_members
        .retain_mut(|member| member.forget_closed_pane(&pane_id));
    group.active_pane = group.active_pane.min(group.panes.len().saturating_sub(1));
    group.pane_grid.replace_tree(group.layout.pane_grid_tree());

    let pane_title = if pane.title.trim().is_empty() {
        group.title.clone()
    } else {
        pane.title.clone()
    };
    let layout = PaneLayout::leaf(pane_id.clone());
    let (
        id,
        title,
        subtitle,
        _icon,
        status,
        _kind,
        _ssh_host_id,
        connection_error,
        pinned,
        history,
    ) = owner.map_or_else(
        || {
            (
                format!("{}-{pane_id}", group.id),
                pane_title,
                group.subtitle.clone(),
                group.icon.clone(),
                group.status,
                group.kind,
                group.ssh_host_id.clone(),
                group.connection_error.clone(),
                false,
                group.in_app_history.clone(),
            )
        },
        |member| {
            (
                format!("{}-{pane_id}", member.session_id),
                member.title,
                member.subtitle,
                member.icon,
                member.status,
                member.kind,
                member.ssh_host_id,
                member.connection_error,
                member.pinned,
                member.in_app_history,
            )
        },
    );
    let kind = pane.endpoint.kind();
    let ssh_host_id = pane.endpoint.profile_id().map(str::to_owned);
    let icon = match kind {
        SessionKind::Local => "terminal",
        SessionKind::Ssh => "server",
    }
    .to_owned();
    let pane_grid = layout.pane_grid_state();
    let detached = Session {
        id,
        title,
        subtitle,
        icon,
        status,
        kind,
        ssh_host_id,
        connection_error,
        pinned,
        detached: false,
        panes: vec![pane],
        active_pane: 0,
        layout,
        pane_grid,
        in_app_history: history,
        group_members: Vec::new(),
    };
    let at = session_idx + 1;
    sessions.insert(at, detached);
    Some(at)
}

/// Resolve one shared-host `ItemDropRequest` into the group the app should
/// apply, or `None` when the drop was not a session-onto-edge drop at all
///.
///
/// # Everything this refuses, and why refusing here is the point
///
/// The drag host publishes any accepted drop; deciding what an accepted drop
/// *means to this app* is the app's job, and it must be one place or the two
/// entry points (pointer drop, and any future keyboard "Group to…") drift.
///
/// * a destination that is neither the content area nor a session-tree
///   collection;
/// * a content target that is not one of the four edge items, including
///   `DropTarget::OntoCollection` from the inert centre;
/// * a sidebar target that is not a named row;
/// * a drag carrying no items, or more than one — a session is grouped
///   singly, and silently taking the first of several would apply an action
///   the user did not ask for;
/// * the active session itself, which `group_into` would refuse as
///   `SameSession` anyway; caught here so the refusal is one rule.
///
/// Returns source id, target id, edge, and the target pane the edge belongs
/// to (`None` for the whole layout). A content-edge drop targets the
/// currently visible session with its explicit direction. A tab-on-tab drop
/// targets the named sidebar row and uses the conventional automatic split to
/// the right.
pub(crate) fn resolve_drop(
    request: &enzui_core::drag::ItemDropRequest,
    active_session_id: &str,
) -> Option<(String, String, GroupEdge, Option<String>)> {
    use enzui_core::drag::DropTarget;
    let [source] = request.request.items.as_slice() else {
        return None;
    };
    let DropTarget::OntoItem(target_item) = &request.target else {
        return None;
    };
    let (target, edge, pane) = if request.request.destination.0 == CONTENT_COLLECTION {
        let (edge, pane) = GroupEdge::from_item_id(target_item)?;
        (active_session_id.to_owned(), edge, pane)
    } else if request.request.destination.0.starts_with("term-sessions-") {
        (target_item.0.clone(), GroupEdge::Right, None)
    } else {
        return None;
    };
    if source.0 == target {
        return None;
    }
    Some((source.0.clone(), target, edge, pane))
}

// Production composition stays beside the geometry/model it exposes.
use crate::{App, Message};
use iced::Element;

impl App {
    /// The terminal content area, wrapped as the session drop zone.
    ///
    /// # Why the zones are keyed items and not a cursor callback
    ///
    /// `enz_drag_item_surface` is the shipped registration wrapper for keyed
    /// drop targets, and the drag host already hit-tests a release against
    /// the rectangles it is given, in the host's own coordinate space. So each
    /// edge of each pane is a keyed item whose rectangle comes from
    /// [`GroupEdge::zone_in`], and whose highlight is the half the session
    /// would take ([`GroupEdge::preview_in`]), derived from the content
    /// area's real laid-out size every event pass. The app never reads a
    /// cursor position, and there is no second hit-test to drift from the
    /// one the user sees.
    ///
    /// This is transparent to the terminal beneath it: the wrapper forwards
    /// every event to its child first, so keystrokes, selection drags inside
    /// the grid and OSC-8 link clicks are untouched. It registers only while
    /// a host is actually hosting a drag (`DragHostChannel::is_hosting`).
    pub(super) fn content(&self) -> Element<'_, Message> {
        use enzui_widgets::drag_item_surface::{DragSurfaceItem, enz_drag_item_surface};

        let body = self.content_body();
        let palette = enzui_widgets::theme_lookup::palette(&self.theme());
        let mut highlight = palette.accent_soft;
        highlight.a *= 0.35;
        let outline = palette.accent;
        let radius = self.enz_theme().radius.md;
        let areas = self.drop_areas();
        let top = self.grid_top_inset();
        let channel = self.drag_channel.clone();
        let active = self.sessions.get(self.active).map(|s| s.id.clone());
        enz_drag_item_surface(
            &self.drag_channel,
            body,
            self::CONTENT_COLLECTION,
            Vec::new(),
        )
        // The callback receives EnzDragItemSurface's own allocation after the
        // shell, sidebar and titlebar have laid out. It is also the geometry
        // the framework paints for the affordance, so neither path uses the
        // window/content-size proxy.
        .items_from_size(move |size| {
            // A session dragged over itself has nowhere to go: no zone, so
            // no preview promising a drop that would be refused.
            let dragging_itself = channel.layout_drag().is_some_and(|drag| {
                drag.items
                    .iter()
                    .any(|item| active.as_deref() == Some(item.0.as_str()))
            });
            if dragging_itself {
                return Vec::new();
            }
            // The panes lie below the strip the floating titlebar covers.
            let grid = iced::Rectangle {
                x: 0.0,
                y: top,
                width: size.width,
                height: (size.height - top).max(0.0),
            };
            areas
                .iter()
                .flat_map(|(pane, unit)| {
                    let area = iced::Rectangle {
                        x: grid.x + unit.x * grid.width,
                        y: grid.y + unit.y * grid.height,
                        width: unit.width * grid.width,
                        height: unit.height * grid.height,
                    };
                    GroupEdge::ALL.into_iter().map(move |edge| {
                        DragSurfaceItem::new(
                            edge.item_id(pane.as_deref()),
                            edge.zone_in(area),
                            edge.label(),
                        )
                        .feedback_bounds(edge.preview_in(area))
                        .drag_source(false)
                        .drop_target(true)
                    })
                })
                .collect()
        })
        .drop_target_highlight(highlight)
        .drop_target_outline(outline, radius)
        .accepts_background(false)
        .into()
    }

    /// The areas a session can be dropped around, as fractions of the pane
    /// area: each pane of a tiled session, or the whole area (`None`) for a
    /// single pane or the narrow layout, which stacks panes instead of tiling.
    fn drop_areas(&self) -> Vec<(Option<String>, iced::Rectangle)> {
        let whole = vec![(
            None,
            iced::Rectangle::new(iced::Point::ORIGIN, iced::Size::new(1.0, 1.0)),
        )];
        let Some(session) = self.sessions.get(self.active) else {
            return whole;
        };
        if session.panes.len() < 2
            || crate::pane_layout::is_narrow_layout_with(
                self.content_pane_width(),
                session.panes.len(),
                &self.enz_theme().sizing,
            )
        {
            return whole;
        }
        crate::pane_layout::flatten_to_rects(&session.layout)
            .into_iter()
            .map(|rect| {
                (
                    Some(rect.pane_id),
                    iced::Rectangle {
                        x: rect.x,
                        y: rect.y,
                        width: rect.w,
                        height: rect.h,
                    },
                )
            })
            .collect()
    }
}

impl App {
    /// Run a grouping request and report its outcome.
    ///
    /// Both halves matter. On success the active selection follows the target
    /// session to its new index (removing the source shifts every index after
    /// it), and the PTY is resized because the pane it lives in just changed
    /// size. On refusal the reason is put in the pane feedback strip rather
    /// than swallowed: every `GroupError` is a state a user can reach, and a
    /// control that does nothing and says nothing is a defect.
    pub(super) fn apply_group_into(
        &mut self,
        target_id: &str,
        source_id: &str,
        edge: self::GroupEdge,
        pane: Option<&str>,
    ) {
        match self::group_into_pane(&mut self.sessions, source_id, target_id, edge, pane) {
            Ok(target_idx) => {
                self.pane_feedback = None;
                self.active = target_idx;
                self.invalidate_sidebar();
                self.resize_active_pty();
            }
            Err(err) => {
                self.pane_feedback = Some(err.message().to_owned());
            }
        }
    }

    /// Take one member back out of a group.
    pub(super) fn apply_ungroup(&mut self, session_idx: usize, member_index: usize) {
        if let Some(restored) = self::ungroup(&mut self.sessions, session_idx, member_index) {
            self.pane_feedback = None;
            // Select the session that just came back: it is the one the user
            // acted on, and leaving the group selected would make a
            // successful ungroup look like nothing happened.
            self.active = restored;
            self.invalidate_sidebar();
            self.resize_active_pty();
        }
    }
}

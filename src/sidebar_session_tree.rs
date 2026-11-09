//! The session sidebar's `enz_tree_view` adapter.
//!
//! Each group is one `enz_tree_view`, and the tree owns:
//!
//! * `Role::Tree` with keyed `Role::TreeItem` rows (one Tab stop per group,
//!   arrow / Home / End roving, Enter activates);
//! * the row surface, hover overlay, selected fill and focus ring;
//! * per-row ids derived from `session.id` through the shared
//!   `tree_row_id` derivation, mirrored by
//!   `context_menu::session_row_id`, so focus restoration after the menu
//!   still lands on the exact session that opened it;
//! * the context-menu affordance under the shared `tree_row_context_id`
//!   derivation, so an AT-routed `ShowContextMenu` reaches the menu without a
//!   pointer. Tests recompute that id through `context_target_id`; production
//!   publishes it directly from `enz_tree_view`.
//!
//! The two-line title/cwd body and the trailing pin/detach accessories
//! stay owned by the app \u2014 supplied through `.row_content(..)`. The
//! renaming row swaps its body for an inline `enz_text_field` bound to
//! the draft; every other row keeps its two-line ramp.
//!
//! # Why one owned entry point, not two lifetime-parametric ones
//!
//! One owned entry point serves both the cached path (`enz_cached_subtree`,
//! whose retained subtree needs `Element<'static>`) and the uncached path: it
//! takes ownership of the rename tuple by clone (uncached path only, one per
//! keystroke) and returns `Element<'static, Message>` from both call sites. A
//! lifetime-borrowing variant cannot compile, because the returned element
//! would outlive the locals it borrows from in `sidebar_body`. See
//! `App::sidebar_body_cached` and `App::sidebar_body` in `main.rs`.

use enzui_core::drag::ItemId;
use enzui_core::virtual_sequence::SequenceKey;
use enzui_widgets::drag_drop_host::DragHostChannel;
use enzui_widgets::drag_item_surface::{DragSurfaceItem, enz_drag_item_surface};
use enzui_widgets::sidebar_content::EnzSidebarContent;
use enzui_widgets::text_field::enz_text_field_owned;
use enzui_widgets::tree_view::{EnzTreeItem, enz_tree_view};
use iced::Element;

use crate::Message;
use crate::model;
use crate::session_row::{
    SidebarSessionSnapshot, session_row_body_branch_height, session_row_body_min_height,
    session_row_body_owned, snapshot_accessible_label,
};

/// The id-prefix namespace both per-section session trees use.
///
/// Shared between the Local and SSH trees. Test helpers recompute row and
/// context ids from this prefix to verify the ids that `enz_tree_view`
/// publishes directly in production. Session ids are globally unique across
/// both kinds, so no two rows can share a key: the derived ids stay unique even
/// though the prefix is shared, and the tree_view's own doc caveat about
/// roots-with-the-same-key does not apply.
pub(crate) const SIDEBAR_SESSION_TREE_PREFIX: &str = "term-sidebar-sessions";

/// Stable widget id of the Local or SSH tree that owns a session row. Row ids
/// address the roving descendant; this id addresses the actual focus stop.
pub(crate) fn session_tree_id(kind: model::SessionKind) -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::from(format!(
        "{SIDEBAR_SESSION_TREE_PREFIX}-{}",
        match kind {
            model::SessionKind::Local => "local",
            model::SessionKind::Ssh => "ssh",
        }
    ))
}

/// The fixed row height, in px, both session trees use.
///
/// Explicit safe fixed-height contract: the row is committed to be at least
/// [`session_row_body_min_height`] tall so the two-line title/cwd body cannot
/// be silently clipped by the tree's row scissor. A caller under-measuring here
/// loses the cwd line without any layout error, so this is pinned once, next to
/// the constant it must dominate, and is asserted by
/// `session_tree_row_height_dominates_the_body_min` in `sidebar_sections`.
#[cfg(test)]
pub(crate) const SIDEBAR_SESSION_TREE_ROW_HEIGHT: f32 =
    SIDEBAR_SESSION_TREE_ROW_SURFACE + SIDEBAR_SESSION_TREE_ROW_GAP;

/// The painted height of one session row's surface, in px.
///
/// This is what 48.0 used to mean before the rail gained a gap: the row's
/// own fill, hover overlay, selected fill and focus ring are this tall, and
/// it is this value — not the pitch — that must dominate
/// [`session_row_body_min_height`], because the body is laid out inside the
/// surface.
pub(crate) const SIDEBAR_SESSION_TREE_ROW_SURFACE: f32 = 48.0;

/// Vertical breathing room between two adjacent session rows, in px
///.
///
/// The review: *"sidebar terminal session rows should have some padding / gap
/// between them"*. Before this the rows abutted — two 48 px two-line ramps
/// with no separation at all, so a glance could not tell where one session
/// ended and the next began.
///
/// 4 px is not an invention: it is the framework's own **row gap** for a
/// sidebar rail, the value `sidebar_content` documents against its larger
/// tokenized section gap (`DEFAULT_SECTION_GAP` = 20 px). Matching it keeps
/// the boundary between two rows lighter than the boundary between two
/// sections, which is the rhythm the rest of the fleet's sidebars already
/// have, and it is `SpacingScale::xs`.
///
/// The gap is applied through the shared
/// [`EnzTreeView::tree_row_gap`](enzui_widgets::tree_view::EnzTreeView::tree_row_gap)
/// capability added for this issue, **not** by padding each row body here: a
/// per-row hand padding would shrink the body inside an unchanged surface
/// (moving the text, not separating the rows) and would leave the hover fill,
/// the selected fill and the focus ring still touching their neighbours.
pub(crate) const SIDEBAR_SESSION_TREE_ROW_GAP: f32 = 4.0;

pub(crate) const SIDEBAR_SESSION_LEAF_DISCLOSURE_SIZE: f32 = enzui_widgets::tree_view::LABEL_GAP;

/// The section of pinned sessions, listed first and closed by a separator.
pub(crate) const PINNED_SECTION: &str = "Pinned";

/// Builder gap that makes the visible surface-to-caption boundary resolve to
/// the framework's section token. Tree surfaces are centred in their fixed
/// pitch, leaving half the row gap after the last painted row; subtract that
/// already-present clearance rather than accidentally producing 22 px from a
/// 20 px contract.
pub(crate) fn section_gap_before_tree_caption(sp: &enzui_theme::SpacingScale) -> f32 {
    (enzui_widgets::sidebar_content::DEFAULT_SECTION_GAP.resolve(sp)
        - SIDEBAR_SESSION_TREE_ROW_GAP / 2.0)
        .max(0.0)
}

/// Assemble every section (pinned, Local, SSH) that has at least one session
/// into `body`: one `sidebar_session_tree` per non-empty group, with a
/// `push_section` caption over the SSH group only.
///
/// Callers own the outer wrapping (`enz_cached_subtree` on the idle frame; the
/// raw scroll view on the rename-active frame). This helper deliberately takes
/// an already-constructed `EnzSidebarContent` and returns the same builder so
/// both callers pushing rhythm through the widget's `push_section` cannot
/// drift; keeping them literally one code path is stronger than the static text
/// scan that gates them.
pub(crate) fn push_session_groups(
    mut body: EnzSidebarContent<'static, Message>,
    snapshots: &[SidebarSessionSnapshot],
    active_id: Option<String>,
    rename: Option<(String, String)>,
    sp: enzui_theme::SpacingScale,
    // The app's durable drag channel. Passed down rather than
    // built here: the same channel must reach the content area's drop zones
    // and this rail, and a channel built per view pass would be a different
    // one from the one the retained sidebar subtree captured.
    drag: &DragHostChannel,
) -> EnzSidebarContent<'static, Message> {
    // A detached tab lives in its own window, which carries its re-dock
    // control; listing it here too showed one terminal twice.
    let listed = |pinned: bool| {
        snapshots
            .iter()
            .enumerate()
            .filter(move |(_, snap)| !snap.in_own_window && snap.pinned == pinned)
            .map(|(i, snap)| (i, snap.clone()))
    };
    // Pinned sessions come first, with no caption, closed by a line.
    let pinned: Vec<(usize, SidebarSessionSnapshot)> = listed(true).collect();
    if !pinned.is_empty() {
        body = body.push(sidebar_session_tree(
            pinned,
            active_id.clone(),
            rename.clone(),
            sp,
            drag,
            PINNED_SECTION,
        ));
        body = body.push(
            iced::widget::container(enzui_widgets::divider::enz_divider())
                .padding(iced::Padding::from([sp.xs(), sp.sm()])),
        );
    }
    for (kind, label) in [
        (model::SessionKind::Local, "Local"),
        (model::SessionKind::Ssh, "SSH"),
    ] {
        let group: Vec<(usize, SidebarSessionSnapshot)> = listed(false)
            .filter(|(_, snap)| snap.kind == kind)
            .collect();
        if group.is_empty() {
            continue;
        }
        // Local sessions are the sidebar's default and need no caption;
        // only remote ones are set apart under theirs.
        if kind != model::SessionKind::Local {
            body = body.push_section(label);
        }
        body = body.push(sidebar_session_tree(
            group,
            active_id.clone(),
            rename.clone(),
            sp,
            drag,
            label,
        ));
    }
    body
}

/// Build one section's session-row tree from owned snapshot data.
///
/// The tree owns the shared custom-row API:
///
/// * one `Role::Tree` group with keyed `Role::TreeItem` rows built from
///   `session.id` (`EnzTreeItem::key`), so `on_select` / `row_on_context`
///   report the row's model index unaffected by scrolling;
/// * `.row_content(..)` supplies only the two-line body via
///   [`session_row_body_owned`]; the tree still owns the row surface, the
///   roving-focus `FocusButton`, the hover overlay, the selected fill, the
///   focus ring, and the `Role::TreeItem`'s selected/level state, the
///   accessibility half a custom body must not duplicate;
/// * `.row_on_context(..)` publishes the row's context-menu affordance under
///   the shared `tree_row_context_id` derivation; tests recompute that id
///   through `context_menu::context_target_id` to verify the AT routing
///   contract;
/// * `.selected_key(session.id)`: selection follows the active session across
///   an insert/remove/reorder rather than being silently transferred to
///   whatever row drifted into the old slot, which a positional index would do.
///
/// When `rename == Some((id, draft))` matches this row's session id, the
/// closure returns an inline `enz_text_field` bound to `draft` in place of the
/// two-line body. Every other row keeps its two-line ramp so the rest of the
/// rail stays operable.
pub(crate) fn sidebar_session_tree(
    group: Vec<(usize, SidebarSessionSnapshot)>,
    active_id: Option<String>,
    rename: Option<(String, String)>,
    sp: enzui_theme::SpacingScale,
    drag: &DragHostChannel,
    section: &str,
) -> Element<'static, Message> {
    let items: Vec<EnzTreeItem> = group
        .iter()
        .map(|(_, snap)| {
            EnzTreeItem::leaf(snap.title.clone(), 0)
                .key(snap.id.clone())
                .aria_label(snapshot_accessible_label(snap))
        })
        .collect();

    let selected_key = active_id
        .as_deref()
        .and_then(|id| group.iter().find(|(_, snap)| snap.id == id))
        .map(|(_, snap)| SequenceKey::new(snap.id.clone()));

    let snaps: Vec<SidebarSessionSnapshot> = group.iter().map(|(_, snap)| snap.clone()).collect();
    // A second owned copy for the trailing-action closure. Both
    // closures must be `'static`, and `row_content` already moves `snaps`.
    let snaps_for_trailing: Vec<SidebarSessionSnapshot> = snaps.clone();
    // A third copy for the drag registration, which outlives
    // neither closure but is consumed after both are built.
    let snaps_for_items: Vec<SidebarSessionSnapshot> = snaps.clone();
    let indices: Vec<usize> = group.iter().map(|(i, _)| *i).collect();
    let ids: Vec<String> = snaps.iter().map(|s| s.id.clone()).collect();
    let rename_id = rename.as_ref().map(|(id, _)| id.clone());
    let rename_draft = rename.as_ref().map(|(_, draft)| draft.clone());

    let tree_id = snaps.first().map_or_else(
        || {
            iced::advanced::widget::Id::from(format!(
                "{SIDEBAR_SESSION_TREE_PREFIX}-empty-{section}"
            ))
        },
        |snapshot| {
            if section == PINNED_SECTION {
                iced::advanced::widget::Id::from(format!("{SIDEBAR_SESSION_TREE_PREFIX}-pinned"))
            } else {
                session_tree_id(snapshot.kind)
            }
        },
    );
    // A section with a git branch on any row gives every row its third line's
    // room; rows without one centre their two lines in it.
    let row_surface = if snaps.iter().any(|snap| snap.branch.is_some()) {
        SIDEBAR_SESSION_TREE_ROW_SURFACE + session_row_body_branch_height()
            - session_row_body_min_height()
    } else {
        SIDEBAR_SESSION_TREE_ROW_SURFACE
    };
    let row_height = row_surface + SIDEBAR_SESSION_TREE_ROW_GAP;
    let mut tree = enz_tree_view::<Message>(items)
        .id(tree_id)
        .id_prefix(SIDEBAR_SESSION_TREE_PREFIX)
        .aria_label(crate::i18n::label("Sessions"))
        .tree_row_height(row_height)
        .tree_row_gap(SIDEBAR_SESSION_TREE_ROW_GAP)
        .disclosure_size(SIDEBAR_SESSION_LEAF_DISCLOSURE_SIZE)
        .selected_key(selected_key)
        // The session list is the sidebar: select with the sidebar's tint.
        .in_sidebar(true)
        .row_context_label("Session actions")
        .row_content(move |idx, _item| {
            let snap = &snaps[idx];
            if rename_id.as_deref() == Some(snap.id.as_str()) {
                let draft = rename_draft.clone().unwrap_or_default();
                // `enz_text_field_owned` because `row_content` must return
                // an `Element<'static, Message>` here and the field's
                // value would otherwise borrow a per-frame local.
                // Edited in place where the title stood: no field box
                // inside the row's own surface.
                return enz_text_field_owned("", "Tab name", draft, Message::SessionRenameEdited)
                    .variant(enzui_widgets::FieldVariant::Inline)
                    .on_submit(Message::CommitSessionRename)
                    .aria_label(crate::i18n::label("Tab name"))
                    .id(crate::focus_ids::session_rename_field_id())
                    .into_element();
            }
            session_row_body_owned(snap, &sp)
        })
        .on_select(move |idx| Message::SelectSession(indices[idx]))
        .row_on_context(move |idx, at| Message::OpenContextMenu(ids[idx].clone(), at));

    // Each row offers its close control (and a group its ungroup control)
    // at the right, shown while the row is hovered or focused, so a session
    // closes from where it is listed rather than from the titlebar.
    tree = tree
        .row_trailing(move |idx, _item| {
            let snap = snaps_for_trailing.get(idx)?;
            let mut actions = iced::widget::row![]
                .spacing(2.0)
                .align_y(iced::Alignment::Center);
            if let Some(member) = snap.group_members.last() {
                let last_index = snap.group_members.len() - 1;
                let label = crate::i18n::format(
                    crate::i18n::localizer(),
                    "Ungroup {} from {}",
                    &[("arg0", &(member)), ("arg1", &(snap.title))],
                );
                actions = actions.push(
                    enzui_widgets::enz_toolbar_icon(
                        "ungroup",
                        Message::UngroupSession(snap.id.clone(), last_index),
                    )
                    .aria_label(label.clone())
                    .tooltip(label)
                    .into_element(),
                );
            }
            let close = crate::i18n::format(
                crate::i18n::localizer(),
                "Close {}",
                &[("arg0", &(snap.title))],
            );
            actions = actions.push(
                enzui_widgets::enz_toolbar_icon(
                    "x",
                    Message::ContextRowActivated(
                        snap.id.clone(),
                        crate::context_menu::RowKind::Close,
                    ),
                )
                .size(20.0, 12.0)
                .aria_label(close.clone())
                .tooltip(close)
                .into_element(),
            );
            // Small, in the row's top-right corner, clear of the text.
            Some(
                iced::widget::container(actions)
                    .height(iced::Length::Fill)
                    .align_y(iced::alignment::Vertical::Top)
                    .padding(iced::Padding::ZERO.top(4.0))
                    .into(),
            )
        })
        .row_trailing_visibility(
            enzui_widgets::list_row_surface::TrailingVisibility::OnHoverOrFocus,
        );
    tree = tree.panel_border_width(0.0);

    let item_geometry = move |size: iced::Size| {
        let gutter = enzui_widgets::list_row_surface::DEFAULT_OUTER_GUTTER;
        snaps_for_items
            .iter()
            .enumerate()
            .map(|(i, snap)| {
                let y = i as f32 * row_height;
                DragSurfaceItem::new(
                    ItemId::new(snap.id.clone()),
                    iced::Rectangle {
                        x: 0.0,
                        y,
                        width: size.width,
                        height: row_surface,
                    },
                    snap.title.clone(),
                )
                .feedback_bounds(iced::Rectangle {
                    x: gutter,
                    y: y + SIDEBAR_SESSION_TREE_ROW_GAP / 2.0,
                    width: (size.width - 2.0 * gutter).max(0.0),
                    height: row_surface,
                })
                .drop_target(true)
            })
            .collect()
    };
    enz_drag_item_surface(
        drag,
        Element::from(tree),
        crate::pane_group::session_collection(section),
        Vec::new(),
    )
    .items_from_size(item_geometry)
    .allows_captured_press(true)
    .accepts_background(false)
    .into()
}

/// A compile-time floor check: the row height the trees pin dominates the
/// minimum body height, so a two-line ramp cannot be silently clipped.
///
/// Kept as a `const fn` next to the two constants it screens so shrinking
/// either one on its own is a mechanical arithmetic error (`cargo check`
/// fails), not a silent visual regression at runtime.
const _: () = {
    // Const-context float comparison via subtraction: `>=` is unstable on
    // `const fn f32` at Rust 1.97, but a static assertion on the sign of
    // the difference is stable.
    //
    // The floor is the row surface, not the pitch. The gap is taken out of the
    // pitch by `tree_row_gap`, so a body sized against the pitch would be
    // clipped by exactly the gap, silently losing the cwd line. Screening the
    // surface makes widening the gap without widening the pitch a compile
    // error.
    let slack = SIDEBAR_SESSION_TREE_ROW_SURFACE - session_row_body_min_height();
    assert!(
        slack >= 0.0,
        "SIDEBAR_SESSION_TREE_ROW_SURFACE must be >= session_row_body_min_height()"
    );
    // And the surface must survive the framework's own clamp, or the gap the
    // rail asks for is not the gap it gets.
    let clamp_slack = SIDEBAR_SESSION_TREE_ROW_SURFACE - enzui_widgets::tree_view::MIN_ROW_SURFACE;
    assert!(
        clamp_slack >= 0.0,
        "SIDEBAR_SESSION_TREE_ROW_SURFACE must clear tree_view::MIN_ROW_SURFACE"
    );
};

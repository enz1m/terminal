//! Right-click context menu for a sidebar **session row**.
//!
//! A right-click on a session row yields an `(id, x, y)` triple, and that is
//! the case this module serves. The anchor is the
//! app's hit-test on a row inside a grouped, scrolled sidebar, not a static
//! child a host could measure for itself.
//!
//! # The point-anchored host, not the bare placement layer
//!
//! The trigger-anchored [`enz_context_menu_host`] cannot express this, because
//! it resolves its anchor from its own child's layout, but
//! [`enz_point_context_menu_host`] is the dual that takes exactly the point the
//! hit-test produced, and `enz_positioned_overlay` is the bare placement layer
//! beneath both.
//!
//! See [`OpenContextMenu`] for what the menu model carries (rows, disabled
//! states, a real Escape) beyond the bare layer.
//!
//! [`enz_context_menu_host`]: enzui_widgets::context_menu_host::enz_context_menu_host
//! [`enz_point_context_menu_host`]: enzui_widgets::context_menu_host::enz_point_context_menu_host
//!
//! # This is a SESSION-ROW menu, not a terminal copy/paste menu
//!
//! The terminal grid has no context menu; a copy/paste menu there would be a
//! separate feature. The selection-dependency question a grid menu would raise still applies here;
//! see the next section, where it lands on Move up / Move down / Close tab
//! instead.
//!
//! # The rows
//!
//! | row | note |
//! |---|---|
//! | `Pin tab` / `Unpin tab` | label AND icon flip on `pinned` |
//! | `Detach tab` / `Re-dock tab` | label flips on `detached` |
//! | group into Left / Right / Up / Down, `Ungroup` | one grouping section |
//! | `Rename` | |
//! | `Move up` | |
//! | `Move down` | |
//! | `Close tab` | |
//!
//! # Disabled, not absent — and why
//!
//! Three rows cannot always act: `Move up` on the first session, `Move down`
//! on the last, and `Close tab` on the only remaining session
//! (`close_active_session` refuses it). An enabled row that changes nothing is
//! a control the user can operate to no effect.
//!
//! The menu **disables** those rows rather than hiding them. Two reasons, and
//! the first is decisive:
//!
//! 1. **A menu that changes shape under the cursor is unusable for its own
//!    primary verb.** `Move up` and `Move down` are the two rows a user
//!    invokes repeatedly. If `Move up` vanished when a tab reached the top, the
//!    row beneath it would slide up under the pointer — so the next click,
//!    aimed at `Move down`, would land on `Rename`. Hiding a row *because the
//!    user just used it* is the worst possible moment to reflow the menu.
//! 2. **The remaining rows stay where muscle memory left them.** The state,
//!    arrange and close commands keep their relative positions around one contiguous
//!    grouping section; the resulting eleven-row menu never changes shape.
//!
//! Absence is the right answer when a row is *meaningless* for the target
//! (sibling remote-app hides `Wake (WoL)` for a profile with no MAC — there is
//! no such thing as waking it). It is the wrong answer when a row is merely
//! *unavailable right now*, which is what all three of these are. `enz_menu`
//! draws a disabled row dimmed and skips it during roving-focus navigation, so
//! a disabled row is neither clickable nor a keyboard trap.

use enzui_core::context_menu::{InvocationKind, Point as CorePoint};
use enzui_core::menu_model::{EntryId, MenuEntryModel, MenuModel};
use enzui_widgets::context_menu_host::{ContextMenuState, FocusRestoreTarget};
use iced::Point;

use crate::App;

/// Width of the menu surface, in px.
///
/// Documented exception: no matching `SizingTokens` rung. A context menu's
/// width is driven by its own longest label (`Re-dock tab`) plus the icon
/// gutter, not by a scale rung: `SizingTokens` has sidebar (260), dialog
/// (360/460/640) and container (480+) widths, none of which describe a popover
/// this narrow. 200 is the hug floor the sibling music-app and remote-app use
/// for the same surface.
pub(crate) const MENU_WIDTH: f32 = 200.0;

/// Dense row height for this eleven-command menu. At the smallest supported
/// 320 px viewport, eleven rows, three separators and surface padding total
/// 287 px, leaving the context host's 8 px edge clearance on both sides.
pub(crate) const MENU_ROW_HEIGHT: f32 = 24.0;
/// Separator inset paired with [`MENU_ROW_HEIGHT`] for the compact menu.
pub(crate) const MENU_SEPARATOR_INSET: f32 = 2.0;

/// A row of the session context menu.
///
/// One enum, so a label and the effect behind it are produced by the same match
/// arm and cannot drift — `Close tab` cannot end up wired to `Rename`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RowKind {
    /// Flip the session's pinned flag.
    TogglePin,
    /// Flip the session's detached flag.
    ToggleDetach,
    /// Begin an inline rename of the session title.
    Rename,
    /// Move the session one place earlier in the sidebar.
    MoveUp,
    /// Move the session one place later in the sidebar.
    MoveDown,
    /// Close the session and its PTYs.
    Close,
    /// Move this session's running terminal into the ACTIVE
    /// session's content area, on the named edge, and record the two as a
    /// group in the sidebar.
    GroupInto(crate::pane_group::GroupEdge),
    /// Take the newest session merged into this one back out
    /// as its own sidebar row.
    ///
    /// Aimed at the *group*, so this row is enabled only on a session that has
    /// members. The per-member control (which member, by name) lives on the
    /// group's sidebar row — see `session_row`; this menu row is the
    /// keyboard/AT-reachable "undo the last group" and is deliberately the
    /// single, unambiguous one.
    Ungroup,
}

/// Separator-delimited groups.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub(crate) enum RowGroup {
    /// Pin / Detach — the two state flags.
    State,
    /// The four group-into-the-content-area edges and Ungroup.
    Group,
    /// Rename / Move up / Move down — reordering and naming.
    Arrange,
    /// Close.
    Destructive,
}

impl RowKind {
    /// Every row. The five grouping commands form one fixed section between state and arrangement;
    /// rows are disabled, never dropped (see the module docs).
    pub(crate) const ALL: [RowKind; 11] = [
        RowKind::TogglePin,
        RowKind::ToggleDetach,
        // The four edges, then ungroup, in their own group so
        // a rule separates them from the arrange rows above and the
        // destructive row below.
        RowKind::GroupInto(crate::pane_group::GroupEdge::Left),
        RowKind::GroupInto(crate::pane_group::GroupEdge::Right),
        RowKind::GroupInto(crate::pane_group::GroupEdge::Up),
        RowKind::GroupInto(crate::pane_group::GroupEdge::Down),
        RowKind::Ungroup,
        RowKind::Rename,
        RowKind::MoveUp,
        RowKind::MoveDown,
        RowKind::Close,
    ];

    /// A stable key for the row — the row's `EntryId` in `enz_menu`. Derived
    /// from the action, never from the position, so a reorder moves the
    /// keyboard cursor with the row.
    pub(crate) fn key(self) -> &'static str {
        match self {
            RowKind::TogglePin => "pin",
            RowKind::ToggleDetach => "detach",
            RowKind::Rename => "rename",
            RowKind::MoveUp => "move-up",
            RowKind::MoveDown => "move-down",
            RowKind::Close => "close",
            RowKind::GroupInto(edge) => match edge {
                crate::pane_group::GroupEdge::Left => "group-left",
                crate::pane_group::GroupEdge::Right => "group-right",
                crate::pane_group::GroupEdge::Up => "group-up",
                crate::pane_group::GroupEdge::Down => "group-down",
            },
            RowKind::Ungroup => "ungroup",
        }
    }

    /// The row's group. Rules go between groups, never inside one.
    pub(crate) fn group(self) -> RowGroup {
        match self {
            RowKind::TogglePin | RowKind::ToggleDetach => RowGroup::State,
            RowKind::GroupInto(_) | RowKind::Ungroup => RowGroup::Group,
            RowKind::Rename | RowKind::MoveUp | RowKind::MoveDown => RowGroup::Arrange,
            RowKind::Close => RowGroup::Destructive,
        }
    }

    /// Only `Close tab` is destructive: closing a tab tears down its PTYs and
    /// every pane's scrollback with them.
    pub(crate) fn destructive(self) -> bool {
        self == RowKind::Close
    }

    /// The row's semantic icon.
    ///
    /// `TogglePin`'s icon flips with the label: an `Unpin tab` row showing a `pin` glyph would read
    /// as "this is pinned" next to a label that says "un-pin it".
    pub(crate) fn icon(self, pinned: bool) -> &'static str {
        match self {
            RowKind::TogglePin => {
                if pinned {
                    "pin-off"
                } else {
                    "pin"
                }
            }
            RowKind::ToggleDetach => "app-window",
            RowKind::Rename => "text-cursor",
            RowKind::MoveUp => "chevron-up",
            RowKind::MoveDown => "chevron-down",
            RowKind::GroupInto(edge) => edge.icon(),
            // `ungroup` is the Lucide mark for splitting a merged shape apart.
            RowKind::Ungroup => "ungroup",
            RowKind::Close => "action.close",
        }
    }

    /// The visible label. Two rows flip on state: `Pin tab` ↔ `Unpin tab` and
    /// `Detach tab` ↔ `Re-dock tab`.
    pub(crate) fn label(self, pinned: bool, detached: bool) -> &'static str {
        match self {
            RowKind::TogglePin => {
                if pinned {
                    crate::i18n::label("Unpin tab")
                } else {
                    crate::i18n::label("Pin tab")
                }
            }
            RowKind::ToggleDetach => {
                if detached {
                    crate::i18n::label("Re-dock tab")
                } else {
                    crate::i18n::label("Detach tab")
                }
            }
            RowKind::Rename => crate::i18n::label("Rename"),
            RowKind::MoveUp => crate::i18n::label("Move up"),
            RowKind::MoveDown => crate::i18n::label("Move down"),
            RowKind::GroupInto(edge) => edge.label(),
            RowKind::Ungroup => crate::i18n::label("Ungroup"),
            RowKind::Close => crate::i18n::label("Close tab"),
        }
    }

    /// Whether the row can act, given everything about the session it is
    /// raised on.
    ///
    /// All callers use this complete context. Keeping the old two-argument
    /// `enabled(index, total)` shim would silently invent "not active, not
    /// grouped, docked" state and let future production code advertise a
    /// grouping command the reducer refuses.
    pub(crate) fn enabled_in(self, ctx: RowContext) -> bool {
        match self {
            RowKind::MoveUp => ctx.index > 0,
            RowKind::MoveDown => ctx.index + 1 < ctx.total,
            RowKind::Close => ctx.total > 1,
            // Grouping moves this session into the one on screen, so it needs
            // (a) another session to move into, which is the active one, and
            // (b) not to be that session itself: `group_into` refuses
            // `SameSession`, and a row that is enabled while the reducer will
            // refuse it is the advertised-but-dead affordance this menu's
            // disabled-not-dropped policy exists to avoid. A detached tab is
            // refused for the same reason: merging it would empty a window the
            // user is looking at.
            RowKind::GroupInto(_) => ctx.total > 1 && !ctx.is_active && !ctx.detached,
            // Only a session that actually swallowed another can give one
            // back. An ordinary `Split pane right` adds a pane and no member,
            // so it correctly leaves this disabled.
            RowKind::Ungroup => ctx.has_members,
            RowKind::TogglePin | RowKind::ToggleDetach | RowKind::Rename => true,
        }
    }
}

/// A context menu that is currently up.
///
/// `at` is the RAW click point. Clamping belongs to the framework host, which
/// measures the laid-out surface against the *live* viewport; pre-clamping here
/// would clamp twice against two different notions of it.
///
/// # Carries the framework host's own two objects
///
/// The menu is mounted on `enz_point_context_menu_host`, not on
/// `enz_positioned_overlay`, the bare placement layer, which places a surface
/// at a point and knows nothing about menus.
///
/// The session rows with both label flips, keyed
/// entries, group separators, a destructive `Close tab`, genuinely disabled
/// rows, roving focus — and, unlike remote-app (whose doc claimed an Escape it
/// never implemented), **a real Escape**, checked ahead of the session-rename
/// and pane-rename handlers. What the bare layer still left missing:
///
/// * **Focus restoration** to the row the menu was raised from. Nothing
///   restored it, so right-click-then-Escape stranded the keyboard.
/// * **A routable `ShowContextMenu` AT node** — the AT substitute for a
///   gesture only a pointer can make. `enz_context_target` was built without
///   an `.id(..)`, and the framework declines to advertise an action it could
///   never deliver, so the menu was strictly pointer-only.
/// * **Escape scoped to the menu.** It worked, but by *hand-maintained
///   precedence*: the menu's arm had to stay physically above two other Escape
///   owners in one long `if` chain. The host claims Escape only while
///   presenting, so the ordering becomes structural.
/// * **One home for the highlight.** `active` sat beside the surface and was
///   driven by a bespoke `ContextMenuNavigate`; two homes for one cursor is how
///   a menu highlights one row while the user is looking at another.
///
/// [`MenuModel`] now owns the rows AND the highlight, and [`ContextMenuState`]
/// owns whether the host presents plus the invocation it anchors on. The host
/// is *controlled*: it reports [`ContextMenuEvent`]s and the app applies them.
///
/// The model lives here rather than being rebuilt per frame because the host
/// borrows `&'a MenuModel` for the element's lifetime, and a per-frame
/// temporary would reset the highlight on every redraw. Keeping the rows
/// truthful is then `App::update`'s reconcile post-pass.
///
/// [`ContextMenuEvent`]: enzui_widgets::context_menu_host::ContextMenuEvent
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OpenContextMenu {
    /// The session whose row was right-clicked, by **id** rather than index:
    /// an index would silently retarget if a session were removed between the
    /// right-click and the activation.
    pub session: String,
    /// Where the cursor was, in viewport coordinates.
    pub at: Point,
    /// The rows, plus the roving highlight arrow navigation moves.
    pub model: MenuModel,
    /// Whether the host is presenting, and the invocation it anchors on.
    pub state: ContextMenuState,
}

impl OpenContextMenu {
    /// Open a menu on `session`, anchored at `at`, over `rows`.
    ///
    /// The invocation is [`InvocationKind::Pointer`] built from the app's own
    /// hit-test point — the ordinary funnel every entry point uses, so there is
    /// no second anchor channel to keep in step with the host's placement.
    ///
    /// `None` when `rows` has no focusable entry: that is the host's own
    /// `is_available` rule, applied here so the app never hands it a model it
    /// would immediately close with `Dismissed(CloseCause::Host)`.
    pub(crate) fn new(session: impl Into<String>, at: Point, rows: MenuModel) -> Option<Self> {
        if !rows.has_focusable() {
            return None;
        }
        let session = session.into();
        let mut state = ContextMenuState::closed();
        state.open_with_restore(
            InvocationKind::Pointer(CorePoint::new(at.x, at.y)),
            FocusRestoreTarget::tree_row_path(
                crate::sidebar_session_tree::SIDEBAR_SESSION_TREE_PREFIX,
                &[session.as_str()],
            ),
        );
        Some(Self {
            session,
            at,
            model: rows,
            state,
        })
    }
}

/// The [`MenuModel`] for the session at `index` of `total` — the rows the
/// framework host renders.
///
/// # Relationship to the host
///
/// This was `surface(app) -> Option<(Point, Element)>`: the app built the whole
/// `enz_menu` surface and handed it to `enz_positioned_overlay` to place.
/// `enz_point_context_menu_host` builds the same `EnzMenu` from this model and
/// adds the menu semantics the bare layer has none of.
///
/// Everything that made the old surface good is preserved and is now **model
/// state** rather than render-time decoration: the row order, both label
/// flips (`Pin`/`Unpin` on `pinned`, `Detach`/`Re-dock` on `detached`), the
/// icon, the group-derived separators (so a dropped row cannot leave a dangling
/// rule), the `destructive` tone on `Close tab`, and the **disabled** flag that
/// makes arrow navigation's skip-disabled behaviour reachable at all.
///
/// The `Message` is deliberately **not** carried on the entry. A controlled
/// host reports `Activated { id, .. }` and the app resolves that id back
/// through [`row_for_id`] against the session's *live* position. Baking the
/// message in would freeze it at the moment the menu opened, so a row whose
/// condition lapsed while the menu was up — `Move up` after the tab was moved
/// to the top by some other route — would still fire.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RowContext {
    /// The session's position in the sidebar.
    pub index: usize,
    /// How many sessions there are.
    pub total: usize,
    /// Whether this session is the one currently filling the content area
    ///: the target of a group, never its source.
    pub is_active: bool,
    /// Whether this session has sessions merged into it.
    pub has_members: bool,
    /// Whether this session is torn out into its own window.
    pub detached: bool,
}

pub(crate) fn menu_model_in(pinned: bool, ctx: RowContext) -> MenuModel {
    let mut entries: Vec<MenuEntryModel> = Vec::new();
    let mut previous: Option<RowGroup> = None;
    for row in RowKind::ALL {
        if let Some(prev) = previous
            && prev != row.group()
        {
            entries.push(MenuEntryModel::separator(format!("rule-{}", row.key())));
        }
        previous = Some(row.group());
        entries.push(
            MenuEntryModel::command(row.key(), row.label(pinned, ctx.detached))
                .icon(row.icon(pinned))
                .destructive(row.destructive())
                .enabled(row.enabled_in(ctx)),
        );
    }
    MenuModel::new(entries)
}

/// The session the open menu was raised on, paired with its live index.
///
/// `None` when no menu is up, or when the session has vanished — a tab closed
/// between the right-click and the next render. A right-click that raced a
/// close must not act against nothing.
fn open_session(app: &App) -> Option<(usize, &crate::model::Session)> {
    let open = app.context_menu.as_ref()?;
    let index = app.sessions.iter().position(|s| s.id == open.session)?;
    Some((index, &app.sessions[index]))
}

/// The [`MenuModel`] for the app's currently-open menu, if any.
pub(crate) fn model_for_open(app: &App) -> Option<MenuModel> {
    let (index, session) = open_session(app)?;
    Some(menu_model_in(session.pinned, row_context(app, index)))
}

/// Everything the rows need to know about the session at `index`. One
/// derivation, used by the live menu and by the two build sites, so a row
/// cannot be enabled on one path and disabled on the other.
pub(crate) fn row_context(app: &App, index: usize) -> RowContext {
    let session = &app.sessions[index];
    RowContext {
        index,
        total: app.sessions.len(),
        is_active: index == app.active,
        has_members: !session.group_members.is_empty(),
        detached: session.detached,
    }
}

/// The [`RowKind`] an activated entry id names, resolved against the session's
/// **current** position.
///
/// `None` for an id that no longer names an **enabled** row — the honest answer
/// rather than a defect: the host already re-resolves an activation against the
/// live surface before reporting it, and this is the app side of the same rule.
/// A separator's id resolves to `None` too; it carries no action and the host
/// never reports one as activated.
///
/// This takes the full [`RowContext`], not `(index, total)`.
/// The menu enables a row from `enabled_in`, and if activation re-checked a
/// two-argument form the two would disagree on exactly the rows this issue
/// added: `Ungroup` (enabled on a group, but `has_members: false` in the short
/// form) would be drawn live and then silently refused on press — the worst
/// shape a menu row can have. One rule, one context.
pub(crate) fn row_for_id(ctx: RowContext, id: &EntryId) -> Option<RowKind> {
    RowKind::ALL
        .into_iter()
        .find(|row| row.key() == id.as_str() && row.enabled_in(ctx))
}

/// The accessible name the menu surface publishes: the session's own title.
///
/// Named after the session, so AT entering the popup hears which tab it is
/// acting on rather than a generic "Menu".
pub(crate) fn menu_label(app: &App) -> Option<String> {
    open_session(app).map(|(_, s)| s.title.clone())
}

/// The row id retained by the context-menu state for focus restoration.
pub(crate) fn menu_invoker_id(app: &App) -> Option<iced::advanced::widget::Id> {
    app.context_menu
        .as_ref()?
        .state
        .restore_target()?
        .invoker_id()
        .cloned()
}

/// The widget id of the sidebar row for the session with this **stable id**.
///
/// # Why this is minted from `session.id` and not the title
///
/// `enz_tree_view` derives each row's routable id from a `path` of
/// host-supplied `EnzTreeItem::key`s under a fixed role. This app's row title
/// is renameable and not unique (two tabs can both be `zsh`, and `Rename` is
/// one of this menu's rows), so a title-derived restore target would collide
/// between siblings and break precisely when the menu's own verb fires. Session
/// ids are unique by construction, and the tree is given
/// `EnzTreeItem::key(session.id)`; this recomputes exactly the id the tree
/// publishes for that row.
///
/// `the_invoker_id_reaches_the_session_row` pins that the two agree by
/// applying this id to the real rendered tree.
pub(crate) fn session_row_id(session_id: &str) -> iced::advanced::widget::Id {
    enzui_widgets::tree_view::tree_row_id(
        crate::sidebar_session_tree::SIDEBAR_SESSION_TREE_PREFIX,
        &[session_id],
    )
}

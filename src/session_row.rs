// No module-level allow.
//! The reusable multi-line sidebar session row.
//!
//! Round one rebuilt this THREE times and nearly every user rejection was
//! placement/alignment here. The contract (dossier R14):
//!   * leading status dot, vertically centred in the selection capsule;
//!   * a title / live-working-directory ramp;
//!   * trailing pin/detach accessories aligned to the title line;
//!   * everything vertically centred in the selection highlight;
//!   * the bare "only a dot" case must not look weird.
//!
//! Built once, here, as a single function the sidebar calls per session.

#[cfg(test)]
use enzui_widgets::enz_text::enz_text;
use enzui_widgets::enz_text::{TextSize, TextTokens, TextTone};
use enzui_widgets::icon::{IconTone, enz_icon};
#[cfg(test)]
use enzui_widgets::list_row_surface::{ListRowSurfaceTokens, enz_list_row_surface};
use enzui_widgets::status_dot::{StatusKind, enz_status_dot};
use iced::advanced::layout::{self, Layout};
use iced::advanced::text::Renderer as _;
use iced::advanced::widget::Tree;
use iced::advanced::{Widget, mouse, renderer};
#[cfg(test)]
use iced::widget::{Space, column, container, row};
#[cfg(test)]
use iced::{Alignment, Padding};
use iced::{Element, Length, Point, Rectangle, Size};

use crate::model::{Session, SessionStatus};

fn status_kind(status: SessionStatus) -> Option<StatusKind> {
    match status {
        // Idle is represented by absence. A grey dot looked like activity and
        // made a brand-new shell appear to need attention.
        SessionStatus::Idle => None,
        // Command activity deliberately uses warning yellow; success is
        // reserved for a completed command waiting to be acknowledged.
        SessionStatus::Running => Some(StatusKind::Running),
        SessionStatus::Ok => Some(StatusKind::Ok),
        SessionStatus::Warning => Some(StatusKind::Warning),
        SessionStatus::Error => Some(StatusKind::Error),
    }
}

#[cfg(test)]
fn status_dot<'a, Message>(status: SessionStatus) -> Element<'a, Message>
where
    Message: 'a,
{
    match status_kind(status) {
        // `.decorative()` suppresses the dot's own AccessKit Label
        // (`StatusKind::label`) because the parent `enz_list_row_surface`
        // already publishes the row as a `Role::ListItem` whose accessible
        // label folds the status in via `accessible_label` ("build — command
        // running" / "… command finished" / "… error"). Without this the same
        // state announces twice — once as the ListItem name and again as a
        // bare `running` / `ok` / `error` Label sibling — which is the exact
        // double-announce `ce9a56b` / `collect.rs` forbid.
        Some(kind) => container(enz_status_dot(kind).decorative())
            .align_y(Alignment::Center)
            .height(Length::Fill)
            .into(),
        None => Space::new().width(0.0).height(0.0).into(),
    }
}

#[cfg(test)]
fn pin_marker<'a, Message>() -> Element<'a, Message>
where
    Message: 'a,
{
    enz_icon("pin")
        .size(14.0)
        .tint_theme_tone(IconTone::Warning)
        .into_element()
}

#[cfg(test)]
fn accessible_label(session: &Session, status: SessionStatus) -> String {
    let mut states = Vec::new();
    if session.pinned {
        states.push("pinned");
    }
    if session.detached {
        states.push("detached");
    }
    match status {
        SessionStatus::Running => states.push("command running"),
        SessionStatus::Ok => states.push("command finished"),
        SessionStatus::Error => states.push("error"),
        SessionStatus::Idle | SessionStatus::Warning => {}
    }
    if states.is_empty() {
        session.title.clone()
    } else {
        format!("{} — {}", session.title, states.join(", "))
    }
}

/// Build one sidebar row. `on_press` selects (and acknowledges) the session.
#[cfg(test)]
pub(crate) fn session_row<'a, Message>(
    session: &'a Session,
    selected: bool,
    on_press: Message,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    session_row_with_spacing(
        session,
        selected,
        on_press,
        &enzui_theme::SpacingScale::default(),
    )
}

/// Build a sidebar row with the host's live shared-theme spacing.
#[cfg(test)]
pub(crate) fn session_row_with_spacing<'a, Message>(
    session: &'a Session,
    selected: bool,
    on_press: Message,
    sp: &enzui_theme::SpacingScale,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    // ---- leading status dot ------------------------------------------------
    let effective_status = if session.connection_error.is_some() {
        SessionStatus::Error
    } else {
        session.status
    };
    let dot = status_dot(effective_status);

    // ---- title / live-location ramp ---------------------------------------
    // The title is always full-strength ink; selection is conveyed by the
    // capsule fill, not by dimming the unselected title.
    let title = enz_text(session.title.clone())
        .size(TextSize::Footnote)
        .tone(TextTone::Default);

    // The title/location ramp gap reads `SpaceToken::Xxs` (2 px =
    // `space1`), the ladder's smallest non-zero rung. was `.spacing(1.0)`.
    //
    // This is a **role adoption, not a coincidence conversion**.
    // `SpaceToken::Xxs` is the framework's hairline-nudge role, and the app's other title/detail ramps use that same
    // rung. The current title/location ramp has one such gap.
    //
    // Guarded by `session_ramp_gap_reads_the_xxs_token`.
    let mut ramp = column![title]
        .spacing(enzui_theme::SpaceToken::Xxs.resolve(sp))
        .width(Length::Fill);

    let location = if let Some(err) = &session.connection_error {
        (err.clone(), TextTone::Danger)
    } else {
        let cwd = session
            .panes
            .get(session.active_pane)
            .map(|pane| pane.cwd.as_str())
            .filter(|cwd| !cwd.is_empty())
            .unwrap_or("Working directory unavailable");
        (cwd.to_owned(), TextTone::Muted)
    };
    ramp = ramp.push(
        enz_text(location.0)
            .size(TextSize::Caption)
            .tone(location.1),
    );

    // ---- trailing accessories (stacked, aligned to the title line) --------
    let mut accessories: Vec<Element<'a, Message>> = Vec::new();
    if session.pinned {
        accessories.push(pin_marker());
    }
    if session.detached {
        accessories.push(
            enz_icon("external-link")
                .size(14.0)
                .tint_theme_tone(IconTone::Muted)
                .into_element(),
        );
    }

    let trailing: Element<'a, Message> = if accessories.is_empty() {
        Space::new().width(0.0).height(0.0).into()
    } else {
        // Stacked, top-aligned to the title line, right-aligned column.
        container(
            column(accessories)
                .spacing(sp.space1)
                .align_x(Alignment::End),
        ) // was 3→2 (nearest token)
        .align_y(Alignment::Start)
        .padding(Padding::from([sp.space1, 0.0])) // was [2.0, 0.0]: xxs vertical
        .into()
    };

    // ---- assemble: [dot] [ramp] [trailing], all vertically centred --------
    let inner = row![dot, container(ramp).width(Length::Fill), trailing,]
        .spacing(sp.sm()) // was 8
        .align_y(Alignment::Center)
        .width(Length::Fill);

    // ---- the selection capsule -------------------------------------------
    //
    // Was a plain `container(..).style(..)` wrapped in
    // `mouse_area(..).on_press(..)`. That shape **cannot render hover at all**:
    // `container`'s style closure receives only `&Theme` and no interaction
    // status (`vendor/iced/widget/src/container.rs`), so the capsule could
    // paint `selected` and nothing else. Every sidebar session row answered the
    // cursor with no visual feedback whatsoever.
    //
    // `enz_list_row_surface` is the framework's surface for exactly this — a
    // custom-content list/sidebar row in a selection set. What it adds that the
    // hand-roll could not express:
    //
    //   * **hover** and **pressed** tones, composited from the single
    //     `Palette::overlay_hover` / `overlay_pressed` source, so they track a
    //     custom palette rather than being absent;
    //   * keyboard: the row is a composed `FocusButton`, so it is a real Tab
    //     stop and activates on Enter/Space — the `mouse_area` version was
    //     pointer-only, so a keyboard user could not select a session;
    //   * a focus-visible ring from `palette.focus_ring`;
    //   * `Role::ListItem` with live `selected` state published to AccessKit,
    //     where the `mouse_area` row was an AT-invisible box;
    //   * activation on a *completed* click (press AND release on the row)
    //     rather than on pointer-down, so a press-then-drag-away cancels.
    //
    // Geometry is preserved deliberately rather than snapped to the widget
    // defaults, so this is a behaviour change and not a visual one:
    //
    //   * `content_height(true)` — the row is a two-line title/location ramp
    //     plus its padding, so it must keep hugging its content. The widget's
    //     default is a **fixed 34 px** single-line row, which would clip the
    //     location line.
    //   * Runtime tokens preserve the custom 8px radius while colors follow
    //     theme switches.
    //   * `selected_background` is untouched — the widget already defaults it
    //     to `surface_selected`, the exact colour the hand-rolled closure
    //     painted.
    //   * no `min_hit` override: the surface's 24 px floor is far below this
    //     row's natural height (a one-line "only a dot" row is already ~28 px
    //     of text plus 12 px of padding), so the floor cannot move the layout.
    let body = container(inner)
        .width(Length::Fill)
        // was [6.0, 8.0]: space3 (6) vertical, sm (8) horizontal.
        .padding(Padding::from([sp.space3, sp.sm()]));

    enz_list_row_surface(body, on_press)
        .content_padding(0.0)
        // An EXPLICIT id minted from the session's stable id, which
        // `EnzListRowSurface` honours ahead of its own label-derived fallback.
        //
        // The fallback would derive from `.label(session.title)`, and titles
        // here are renameable — by `Rename`, one of the context menu's own rows
        // — and not unique: two tabs can both be "zsh". A title-derived id
        // would therefore collide between siblings and break the menu's focus
        // restoration at exactly the moment the menu's own verb fired.
        .id(crate::context_menu::session_row_id(&session.id))
        .selected(selected)
        .selectable(true)
        .content_height(true)
        .width(Length::Fill)
        .label(accessible_label(session, effective_status))
        .tokens_resolved(|theme: &iced::Theme| {
            let p = enzui_widgets::theme_lookup::palette(theme);
            ListRowSurfaceTokens {
                // The capsule's own 8px radius, not the widget's 10px default.
                row_radius: 8.0,
                // The sidebar's selection tint, as every app's sidebar rows.
                selected_background: enzui_widgets::sidebar_row::selected_tint(&p),
                ..ListRowSurfaceTokens::from_palette(&p)
            }
        })
        .into()
}

// ---------------------------------------------------------------------------
// An owned variant for use inside `enz_cached_subtree`.
//
// `enz_cached_subtree` retains an `Element<'static, ..>` across frames, so
// nothing inside the boundary can borrow from frame-local state. This struct
// carries exactly the sidebar-visible fields of a `Session` (no transcript,
// no pane tree), and the builder produces an `Element<'static, Message>` from
// it. The per-cold-frame clone cost is a handful of small strings — negligible
// next to the 30-widget rebuild it replaces.
// ---------------------------------------------------------------------------

/// The sidebar-visible subset of [`Session`], cheaply cloneable.
#[derive(Clone, Debug)]
pub(crate) struct SidebarSessionSnapshot {
    pub id: String,
    pub title: String,
    pub kind: crate::model::SessionKind,
    pub status: SessionStatus,
    pub(crate) window_focused: bool,
    pub pinned: bool,
    pub detached: bool,
    /// Detached into a window that is open, so not listed in the sidebar.
    pub(crate) in_own_window: bool,
    /// The git branch the active pane's directory is on.
    pub(crate) branch: Option<String>,
    pub cwd: String,
    pub connection_error: Option<String>,
    /// The titles of the sessions merged into this one,
    /// oldest first. Empty for an ordinary session.
    ///
    /// Titles rather than ids because this is what the row *shows*: the group
    /// badge counts them and the ungroup control names the one it will give
    /// back, so a user is never asked to undo an anonymous "member 2".
    pub group_members: Vec<String>,
}

/// The natural minimum height, in px, of the two-line session-row body —
/// title (Footnote) + Xxs gap + cwd (Caption) — padded top and bottom by
/// `SpacingScale::space3` (6 px stock) so the ramp visibly centres inside
/// a fixed-height tree row.
///
/// The tree_view's row surface pins each row to `tree_row_height` and clips at
/// its content box, so a row height smaller than this floor silently loses the
/// cwd line. The caller must pass `tree_row_height(x)` with `x >=
/// session_row_body_min_height()`.
///
/// Not "the whole tree row": tree_view supplies the roving-focus surface,
/// the selection fill, the hover overlay, the focus ring, the ListItem/TreeItem
/// AT node and its selected/level state. This is only the two-line body
/// (leading status dot, title/cwd ramp, trailing pin/detach accessories)
/// that the surface hosts.
/// The body's height with a third (git branch) line.
pub(crate) fn session_row_body_branch_height() -> f32 {
    session_row_body_min_height() + TextSize::Caption.line_height_px() + 2.0
}

pub(crate) const fn session_row_body_min_height() -> f32 {
    // Empirically measured against the current `session_row_owned`'s natural
    // height with the two-line ramp + trailing icons at the default spacing
    // ladder. Kept as a constant rather than a runtime measurement because a
    // dynamic max would rebuild every frame and hide a regression that
    // shrank the body's contribution.
    44.0
}

/// Build ONLY the two-line session-row body — leading status dot, title/cwd
/// ramp, trailing pin/detach accessories — as an `Element<'static, Message>`
/// suitable for [`enz_tree_view`](enzui_widgets::tree_view::enz_tree_view)'s
/// `.row_content(..)` closure.
///
/// # Why this is a body, not a whole row
///
/// The tree_view already owns:
///
/// * the row surface (hover/pressed overlays, selected fill, focus ring),
/// * the roving-focus `FocusButton` and its activation carrier,
/// * the `Role::TreeItem` node with `level` / `selected` / `set_position`,
/// * the routable row id (derived from the row's key) and, when
///   `.row_on_context(..)` is set, the `ShowContextMenu` affordance id,
/// * horizontal padding via `row_padding_x` and vertical centring inside
///   the fixed row height.
///
/// Composing another `enz_list_row_surface` inside would give the tree two
/// competing activation surfaces and two `Role::ListItem` / `Role::TreeItem`
/// nodes for one row. This function returns the content only.
///
/// The dot uses `.decorative()` (as in `session_row`) so its `StatusKind`
/// label does not duplicate the row's own accessible name.
pub(crate) fn session_row_body_owned<Message>(
    snap: &SidebarSessionSnapshot,
    sp: &enzui_theme::SpacingScale,
) -> Element<'static, Message>
where
    Message: 'static,
{
    let effective_status = snapshot_status(snap);
    let (location, location_tone) = if let Some(ref err) = snap.connection_error {
        (err.clone(), TextTone::Danger)
    } else if snap.cwd.is_empty() {
        ("Working directory unavailable".to_owned(), TextTone::Muted)
    } else {
        (snap.cwd.clone(), TextTone::Muted)
    };

    Element::new(SessionRowBody {
        title: snap.title.clone(),
        location,
        location_tone,
        branch: snap.branch.clone(),
        status: effective_status.and_then(status_kind),
        detached: snap.detached,
        spacing: *sp,
    })
}

/// Paint-only body hosted by `tree_view`'s semantic and interactive row.
///
/// `tree_view` already owns the row's `Role::TreeItem`, focus target, pointer
/// routing, selection fill, clipping and fixed-height layout. Building this
/// non-interactive decoration as six nested widget-tree nodes repeated those
/// responsibilities for every session. This leaf paints the same status,
/// title, location and state marks in one node; the parent remains the sole
/// semantic/action owner.
struct SessionRowBody {
    title: String,
    location: String,
    location_tone: TextTone,
    branch: Option<String>,
    status: Option<StatusKind>,
    detached: bool,
    spacing: enzui_theme::SpacingScale,
}

impl SessionRowBody {
    fn height(&self) -> f32 {
        if self.branch.is_some() {
            session_row_body_branch_height()
        } else {
            session_row_body_min_height()
        }
    }
}

impl<Message: 'static> Widget<Message, iced::Theme, iced::Renderer> for SessionRowBody {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::stateless()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::None
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(self.height()))
    }

    fn layout(&mut self, tree: &mut Tree, _renderer: &iced::Renderer, limits: &layout::Limits) {
        tree.size = limits.resolve(Length::Fill, Length::Fixed(self.height()), Size::ZERO);
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let palette = enzui_widgets::theme_lookup::palette(theme);
        let text = TextTokens::from_palette(&palette);
        let dot_size = 8.0;
        // The status dot takes room only when there is one to show.
        let text_x = if self.status.is_some() {
            bounds.x + dot_size + self.spacing.sm()
        } else {
            bounds.x
        };
        let accessory_width = if self.detached {
            14.0 + self.spacing.sm()
        } else {
            self.spacing.sm()
        };
        let text_width = (bounds.width - (text_x - bounds.x) - accessory_width).max(0.0);
        let gap = enzui_theme::SpaceToken::Xxs.resolve(&self.spacing);
        let branch_line = if self.branch.is_some() {
            gap + TextSize::Caption.line_height_px()
        } else {
            0.0
        };
        let ramp_height = TextSize::Footnote.line_height_px()
            + gap
            + TextSize::Caption.line_height_px()
            + branch_line;
        let ramp_y = bounds.center_y() - ramp_height / 2.0;

        if let Some(kind) = self.status {
            let dot: Element<'_, Message> = enz_status_dot(kind).decorative().into();
            draw_session_decoration(
                dot,
                renderer,
                theme,
                style,
                Point::new(bounds.x, bounds.center_y() - dot_size / 2.0),
                Size::new(dot_size, dot_size),
                *viewport,
            );
        }

        draw_session_text(
            renderer,
            &self.title,
            Point::new(text_x, ramp_y),
            Size::new(text_width, TextSize::Footnote.line_height_px()),
            TextSize::Footnote,
            text.default_color,
            *viewport,
        );
        let location_y = ramp_y
            + TextSize::Footnote.line_height_px()
            + enzui_theme::SpaceToken::Xxs.resolve(&self.spacing);
        let location_color = match self.location_tone {
            TextTone::Danger => text.danger_color,
            _ => text.muted_color,
        };
        draw_session_text(
            renderer,
            &self.location,
            Point::new(text_x, location_y),
            Size::new(text_width, TextSize::Caption.line_height_px()),
            TextSize::Caption,
            location_color,
            *viewport,
        );
        if let Some(branch) = &self.branch {
            let line = TextSize::Caption.line_height_px();
            let branch_y = location_y + line + gap;
            let icon_size = 12.0;
            let icon: Element<'_, Message> = enz_icon("git-branch")
                .size(icon_size)
                .tint_theme_tone(IconTone::Muted)
                .into_element();
            draw_session_decoration(
                icon,
                renderer,
                theme,
                style,
                Point::new(text_x, branch_y + (line - icon_size) / 2.0),
                Size::new(icon_size, icon_size),
                *viewport,
            );
            let branch_x = text_x + icon_size + 4.0;
            draw_session_text(
                renderer,
                branch,
                Point::new(branch_x, branch_y),
                Size::new((text_width - (branch_x - text_x)).max(0.0), line),
                TextSize::Caption,
                text.muted_color,
                *viewport,
            );
        }

        let accessory_x = bounds.x + bounds.width - 14.0;
        // A detached tab's mark, centred on the row. Pinned tabs carry no
        // mark: they are listed first, above a separator.
        let accessory_y = bounds.center_y() - 7.0;
        if self.detached {
            let icon: Element<'_, Message> = enz_icon("external-link")
                .size(14.0)
                .tint_theme_tone(IconTone::Muted)
                .into_element();
            draw_session_decoration(
                icon,
                renderer,
                theme,
                style,
                Point::new(accessory_x, accessory_y),
                Size::new(14.0, 14.0),
                *viewport,
            );
        }
    }
}

fn draw_session_text(
    renderer: &mut iced::Renderer,
    content: &str,
    position: Point,
    bounds: Size,
    size: TextSize,
    color: iced::Color,
    viewport: Rectangle,
) {
    renderer.fill_text(
        iced::advanced::text::Text {
            content: content.to_owned(),
            bounds,
            size: size.px().into(),
            line_height: iced::widget::text::LineHeight::Absolute(size.line_height_px().into()),
            font: renderer.font(),
            align_x: iced::advanced::text::Alignment::Left,
            align_y: iced::alignment::Vertical::Top,
            shaping: iced::widget::text::Shaping::Advanced,
            wrapping: iced::widget::text::Wrapping::None,
            ellipsis: iced::widget::text::Ellipsis::End,
            hint_factor: None,
        },
        position,
        color,
        viewport,
    );
}

fn draw_session_decoration<Message>(
    mut element: Element<'_, Message>,
    renderer: &mut iced::Renderer,
    theme: &iced::Theme,
    style: &renderer::Style,
    origin: Point,
    size: Size,
    viewport: Rectangle,
) where
    Message: 'static,
{
    // These decorations are stateless and non-interactive. Drawing their
    // existing framework widgets through a transient local tree preserves the
    // exact SVG/status-dot paint while keeping them out of the retained app
    // tree; the enclosing TreeItem already owns their semantics and events.
    let mut tree = Tree::new(element.as_widget());
    element.as_widget_mut().diff(&mut tree);
    let limits = layout::Limits::new(size, size);
    element.as_widget_mut().layout(&mut tree, renderer, &limits);
    element.as_widget().draw(
        &tree,
        renderer,
        theme,
        style,
        Layout::new(tree.size).move_to(origin),
        mouse::Cursor::Unavailable,
        &viewport,
    );
}

/// The tree-friendly accessible label for one snapshot (`session.title — states`).
///
/// Same rule as `session_row_owned` uses when it calls
/// `enz_list_row_surface(..).label(..)`, exposed so the tree_view host
/// (which owns the row surface here) can feed the same string into
/// `EnzTreeItem::aria_label`.
pub(crate) fn snapshot_accessible_label(snap: &SidebarSessionSnapshot) -> String {
    accessible_label_snapshot(snap)
}

impl SidebarSessionSnapshot {
    /// Extract the sidebar-visible data from a full session.
    pub(crate) fn from_session(s: &Session) -> Self {
        Self {
            id: s.id.clone(),
            title: s.title.clone(),
            kind: s.kind,
            status: s.status,
            window_focused: false,
            pinned: s.pinned,
            detached: s.detached,
            in_own_window: false,
            branch: None,
            // A grouped row has no single honest working directory. Showing
            // the focused pane's cwd made the sidebar claim that location for
            // every terminal in the group. Replace it with topology metadata;
            // ordinary rows keep their live cwd unchanged.
            // A grouped row has no single honest working directory. Showing
            // the focused pane's cwd made the sidebar claim that location for
            // every terminal in the group. Replace it with topology metadata;
            // ordinary rows keep their live cwd unchanged.
            cwd: if s.group_members.is_empty() {
                s.panes
                    .get(s.active_pane)
                    .map(|p| p.cwd.clone())
                    .unwrap_or_default()
            } else {
                format!("{} terminals grouped", s.panes.len())
            },
            connection_error: s.connection_error.clone(),
            group_members: s.group_members.iter().map(|m| m.title.clone()).collect(),
        }
    }

    pub(crate) fn window_focused(mut self, focused: bool) -> Self {
        self.window_focused = focused;
        self
    }

    pub(crate) fn in_own_window(mut self, in_own_window: bool) -> Self {
        self.in_own_window = in_own_window;
        self
    }

    pub(crate) fn branch(mut self, branch: Option<String>) -> Self {
        self.branch = branch;
        self
    }
}

fn snapshot_status(snap: &SidebarSessionSnapshot) -> Option<SessionStatus> {
    if snap.window_focused {
        None
    } else if snap.connection_error.is_some() {
        Some(SessionStatus::Error)
    } else {
        Some(snap.status)
    }
}

fn accessible_label_snapshot(snap: &SidebarSessionSnapshot) -> String {
    let status = snapshot_status(snap);
    let mut states = Vec::new();
    if snap.pinned {
        states.push("pinned");
    }
    if snap.detached {
        states.push("detached");
    }
    match status {
        Some(SessionStatus::Running) => states.push("command running"),
        Some(SessionStatus::Ok) => states.push("command finished"),
        Some(SessionStatus::Error) => states.push("error"),
        Some(SessionStatus::Idle | SessionStatus::Warning) | None => {}
    }
    if states.is_empty() {
        snap.title.clone()
    } else {
        format!("{} — {}", snap.title, states.join(", "))
    }
}

/// Build a sidebar row from owned data, producing `Element<'static, Message>`.
///
/// Used inside `enz_cached_subtree`'s closure, where the retained
/// element must outlive any single frame's borrows.
#[cfg(test)]
pub(crate) fn session_row_owned<Message>(
    snap: SidebarSessionSnapshot,
    selected: bool,
    on_press: Message,
    sp: &enzui_theme::SpacingScale,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let dot: Element<'static, Message> =
        status_dot(snapshot_status(&snap).unwrap_or(SessionStatus::Idle));

    let title = enz_text(snap.title.clone())
        .size(TextSize::Footnote)
        .tone(TextTone::Default);

    let mut ramp = column![title]
        .spacing(enzui_theme::SpaceToken::Xxs.resolve(sp))
        .width(Length::Fill);

    let location = if let Some(ref err) = snap.connection_error {
        (err.clone(), TextTone::Danger)
    } else if snap.cwd.is_empty() {
        ("Working directory unavailable".to_owned(), TextTone::Muted)
    } else {
        (snap.cwd.clone(), TextTone::Muted)
    };
    ramp = ramp.push(
        enz_text(location.0)
            .size(TextSize::Caption)
            .tone(location.1),
    );

    let mut accessories: Vec<Element<'static, Message>> = Vec::new();
    if snap.pinned {
        accessories.push(pin_marker());
    }
    if snap.detached {
        accessories.push(
            enz_icon("external-link")
                .size(14.0)
                .tint_theme_tone(IconTone::Muted)
                .into_element(),
        );
    }

    let trailing: Element<'static, Message> = if accessories.is_empty() {
        Space::new().width(0.0).height(0.0).into()
    } else {
        container(
            column(accessories)
                .spacing(sp.space1)
                .align_x(Alignment::End),
        )
        .align_y(Alignment::Start)
        .padding(Padding::from([sp.space1, 0.0]))
        .into()
    };

    let inner = row![dot, container(ramp).width(Length::Fill), trailing,]
        .spacing(sp.sm())
        .align_y(Alignment::Center)
        .width(Length::Fill);

    let body = container(inner)
        .width(Length::Fill)
        .padding(Padding::from([sp.space3, sp.sm()]));

    let label = accessible_label_snapshot(&snap);
    let row_id = crate::context_menu::session_row_id(&snap.id);
    enz_list_row_surface(body, on_press)
        .content_padding(0.0)
        .id(row_id)
        .selected(selected)
        .selectable(true)
        .content_height(true)
        .width(Length::Fill)
        .label(label)
        .tokens_resolved(|theme: &iced::Theme| {
            let p = enzui_widgets::theme_lookup::palette(theme);
            ListRowSurfaceTokens {
                row_radius: 8.0,
                // The sidebar's selection tint, as every app's sidebar rows.
                selected_background: enzui_widgets::sidebar_row::selected_tint(&p),
                ..ListRowSurfaceTokens::from_palette(&p)
            }
        })
        .into()
}

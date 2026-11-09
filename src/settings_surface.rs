//! Adaptive Settings presentation lifecycle for terminal-emulator.
//!
//! Owns the `ModalPresentation<()>` retention, the probe/render presentation
//! resolver and the modal-host placement/settlement wiring, keeping the
//! ownership out of `main.rs` so the touched large owner does not simply
//! grow. The rendered builder (`crate::App::settings_dialog`) and its nav
//! probe (`crate::App::settings_dialog_nav`) both call
//! [`window_size`] and [`resolved_presentation`] so a desktop dialog and a
//! narrow / short-wide sheet cannot be resolved differently by the two
//! sides. Reverting either the probe or the render adaptive branch reddens
//! the fleet census.
//!
//! The lifecycle is calendar's `sync_settings_presentation` shape:
//!
//! * `settings_open` ⇔ the presentation has an **open** generation.
//! * On `Dismissed` (Escape / scrim / close-X) the presentation transitions
//!   to `closing`, the host keeps rebuilding the outgoing element for one
//!   more paint via `presentation_open(false)`, and focus/AT are withdrawn
//!   in the same frame.
//! * `SettingsExitSettled(key)` retires only the generation that painted
//!   this frame — a rapid reopen mints a new key that the previous close's
//!   late settlement no longer matches.
//! * Any peer overlay (SSH manager, destructive delete confirm) taking the
//!   host retires the outgoing generation immediately so its late
//!   settlement cannot resurrect a superseded dialog.

use enzui_widgets::modal_host::{ModalPresentationKey, SurfacePlacement};
use enzui_widgets::settings_dialog::{EnzSettingsPresentation, enz_settings_presentation_for};

use crate::App;

/// The live window size the settings surface's adaptive resolver reads.
///
/// Terminal boots with `content_size == iced::Size::ZERO` before the first
/// compositor `Resized`, so a bare `content_size` would resolve to a zero-width
/// sheet on frame 1. The 1400×900 fallback keeps the boot frame's dialog
/// centred until the real geometry arrives.
pub(crate) fn window_size(app: &App) -> iced::Size {
    if app.content_size.width <= 0.5 || app.content_size.height <= 0.5 {
        iced::Size::new(1400.0, 900.0)
    } else {
        app.content_size
    }
}

/// The concrete presentation the adaptive resolver picks for the live
/// window. Used by [`placement`] and by the view's modal host so a sheet
/// slides to the bottom edge while a dialog scales in place.
pub(crate) fn resolved_presentation(app: &App) -> EnzSettingsPresentation {
    enz_settings_presentation_for(window_size(app))
}

/// Where the modal host must attach the settings surface for the resolved
/// presentation.
pub(crate) fn placement(app: &App) -> SurfacePlacement {
    match resolved_presentation(app) {
        EnzSettingsPresentation::Sheet => SurfacePlacement::BottomEdge,
        // `resolved_presentation` never returns `Adaptive` (it always picks
        // `Sheet` or `Dialog` from the live size); the arm exists so a
        // future variant is called out rather than silently defaulted.
        EnzSettingsPresentation::Dialog | EnzSettingsPresentation::Adaptive => {
            SurfacePlacement::Centred
        }
    }
}

/// Whether an outgoing settings surface must still be rebuilt so its exit
/// is visible. Read by `view` to keep the dialog element in the modal host
/// after the logical `settings_open` flag has flipped false.
pub(crate) fn is_closing(app: &App) -> bool {
    app.settings_presentation.is_closing()
}

/// The generation a settlement callback must carry to retire the currently
/// retained surface. `None` when no generation is presented.
pub(crate) fn presentation_key(app: &App) -> Option<ModalPresentationKey> {
    app.settings_presentation.key()
}

/// Retire the settled generation, ignoring a stale one. `finish_exit` is
/// generation-safe by construction — a rapid reopen mints a new key that
/// the previous close's late settlement no longer matches, so the new
/// dialog is not torn down.
pub(crate) fn finish_exit(app: &mut App, key: ModalPresentationKey) {
    let _ = app.settings_presentation.finish_exit(key);
}

/// Mirror the retained settings presentation against `settings_open` and
/// stacking overlays. Follows calendar's `sync_settings_presentation`
/// invariant.
pub(crate) fn sync(app: &mut App) {
    if app.settings_open {
        if !app.settings_presentation.is_open() {
            let _ = app.settings_presentation.present(());
        }
        return;
    }
    if app.settings_presentation.is_open() {
        let _ = app.settings_presentation.dismiss();
    }
    // Any peer overlay taking the host retires the outgoing settings
    // generation now, so its late settlement cannot resurrect a dialog the
    // user already dismissed.
    if (app.ssh_manager_open || app.pending_delete_ssh_host.is_some())
        && let Some(key) = app.settings_presentation.key()
    {
        let _ = app.settings_presentation.finish_exit(key);
    }
}

// The rendered Settings surface builder + nav probe.
//
// Kept as **free functions** taking `&App` (not methods on `App`) because the
// existing `settings_surface::window_size`, `resolved_presentation`,
// `placement`, `is_closing`, `presentation_key`, `finish_exit` and `sync`
// helpers already have this shape, and this keeps the settings surface's
// full ownership in one module.

use enzui_widgets::combo_box::enz_combo_box;
use enzui_widgets::editable_shortcuts::{
    EditableShortcutsEvent, editable_shortcuts_reset_all_action, enz_editable_shortcuts,
    shortcuts_settings_page,
};
use enzui_widgets::preferences_group::enz_preferences_group;
use enzui_widgets::preferences_row::enz_preferences_row;
use enzui_widgets::settings_dialog::{SettingsPage, enz_settings_dialog, settings_action};
use iced::widget::{Space, column};
use iced::{Element, Length};

use crate::{
    APPEARANCE_TITLE, Message, SETTINGS_ID_PREFIX, SETTINGS_RESET_APPEARANCE_ID, SETTINGS_ROWS,
    SETTINGS_TITLE, color_scheme, model, settings_row,
};

pub(crate) fn settings_dialog_nav(app: &App) -> enzui_widgets::settings_dialog::SettingsNav {
    use enzui_widgets::settings_dialog::DialogSize;
    let window = window_size(app);
    // Keep the probe's builder in structural lockstep with the
    // render builder below (same `DialogSize`, same page set, same
    // `.presentation(Adaptive)`). Presentation does NOT affect
    // `resolved_nav()` — `EnzSettingsDialog::resolved_nav` reads
    // `settings_dialog_geometry_sized(window, size)` and the label metrics,
    // not the presentation field, so at Terminal's two-page label set both
    // resolver arms return `SettingsNav::Tabs`/`Stacked` identically for a
    // given (window, size). What matters here is CENSUS parity: any auditor
    // that scans callers for `.presentation(Adaptive)` must find it on
    // every builder that resolves settings geometry, or a future change
    // that drops it from the render side goes undetected.
    enz_settings_dialog::<Message>(true, window)
        .size(DialogSize::Small)
        .presentation(EnzSettingsPresentation::Adaptive)
        .page(
            SettingsPage::new("appearance", crate::i18n::label("Appearance")).icon("settings"),
            Space::new(),
        )
        .page(shortcuts_settings_page(), Space::new())
        .resolved_nav()
}

pub(crate) fn settings_dialog(app: &App) -> Element<'_, Message> {
    let enz = app.enz_theme();
    let sp = enz.spacing;
    let schemes: Vec<String> = color_scheme::SCHEMES
        .iter()
        .map(|s| s.name.to_owned())
        .collect();
    let current_scheme = color_scheme::find_by_id(app.settings.resolved_color_scheme())
        .name
        .to_owned();

    let scheme_pick = enz_combo_box("", schemes, Some(current_scheme), |name: String| {
        let id = color_scheme::SCHEMES
            .iter()
            .find(|s| s.name == name)
            .map_or_else(|| color_scheme::DEFAULT_ID.into(), |s| s.id.to_owned());
        Message::SetColorScheme(id)
    })
    .placeholder(crate::i18n::text(SETTINGS_ROWS[0]));

    let cursor_values = [
        ("block", crate::i18n::label("Block")),
        ("underline", crate::i18n::label("Underline")),
        ("bar", crate::i18n::label("Bar")),
    ];
    let cursor_styles = cursor_values
        .iter()
        .map(|(_, label)| (*label).to_owned())
        .collect();
    let cursor_pick = enz_combo_box(
        "",
        cursor_styles,
        Some(
            cursor_values
                .iter()
                .find(|(value, _)| *value == app.settings.resolved_cursor_style())
                .unwrap()
                .1
                .to_owned(),
        ),
        move |label: String| {
            Message::SetCursorStyle(
                cursor_values
                    .iter()
                    .find(|(_, text)| *text == label)
                    .map_or("block", |(value, _)| *value)
                    .to_owned(),
            )
        },
    )
    .placeholder(crate::i18n::text(SETTINGS_ROWS[2]));

    // ---- Main settings group (scheme + cursor) ----------------------
    let main_group = enz_preferences_group(
        column![
            settings_row(crate::i18n::label(SETTINGS_ROWS[0]), scheme_pick),
            settings_row(crate::i18n::label(SETTINGS_ROWS[2]), cursor_pick),
        ]
        .spacing(sp.xs()),
    )
    .title(crate::i18n::label(SETTINGS_TITLE))
    .spacing_scale(sp)
    .typography_scale(enz.typography);

    // ---- Appearance group --------------------------------------------
    // A real bounded pixel value, not three typography-token opinions.
    // The displayed text and the geometry both read the clamped model value.
    let font_size_label = app.settings.font_size_value_label();
    let font_size_stepper = enzui_widgets::stepper::enz_stepper(
        app.settings.clamped_font_size_px(),
        Message::SetFontSizePx,
    )
    .min(model::FONT_SIZE_MIN_PX)
    .max(model::FONT_SIZE_MAX_PX)
    .step(1)
    .value_text(font_size_label)
    .id(iced::advanced::widget::Id::new(
        "terminal-emulator/settings/font-size",
    ))
    // The row paints the title; a stepper caption repeated it above the control.
    .aria_label(crate::i18n::label(SETTINGS_ROWS[1]))
    .into_element();
    let font_size_row = enz_preferences_row(crate::i18n::label(SETTINGS_ROWS[1]))
        .control(font_size_stepper)
        .into_element();

    // The "Font family (informational)" row was removed. It read
    // `Settings::font_family` and rendered its value in a subtitle, but
    // the value never reached the grid — `CellGeometry::tokens()` in
    // `grid_view.rs` pins `TextGridTokens { font: None, .. }` because the
    // paint pitch must match the framework's monospace advance estimate
    // (`estimate_advance_ratio(Monospace)`) that also drives the PTY
    // resize `cols`. Overriding the grid font would need a metric probe
    // per family plus a framework text_grid API that has neither an
    // `.aria_label` on the caller-supplied font path nor a per-family
    // measurement hook (searched `docs/widgets.md` for `font family`,
    // `text_grid` and reviewed `TextGridTokens.font`). A row whose value
    // manages nothing is worse than absent; the field is
    // deleted with the row.

    // Scrollback line count. The row TITLE advertises the accepted range
    // — `Scrollback lines (500 – 20 000)` — built from the SAME clamp
    // bounds the model enforces (`model::SCROLLBACK_BOUNDS_LABEL`, derived
    // from `SCROLLBACK_MIN/MAX`), so the copy can't promise a range the
    // clamp doesn't honour.
    //
    // Was an informational `enz_preferences_row` with a subtitle
    // but NO writable control. `Settings::scrollback_lines` had no
    // editable path and the backend (`emulator.rs`) hardcoded a
    // 5000-line eviction. The row is now backed by an `enz_stepper`
    // wired to `Message::SetScrollback`, whose update arm clamps the
    // value and calls `PtySession::set_max_scrollback` on every live PTY
    // so the ring shrinks/grows immediately.
    let scrollback_stepper = enzui_widgets::stepper::enz_stepper(
        app.settings.clamped_scrollback(),
        Message::SetScrollback,
    )
    .min(model::SCROLLBACK_MIN)
    .max(model::SCROLLBACK_MAX)
    // Step of 500 keeps the ends of the range reachable in a reasonable
    // number of ± presses (500..=20 000 in 40 steps). AT clients that
    // want a specific intermediate value can drive `Action::SetValue`
    // through the SpinButton semantics.
    .step(500)
    // #4 doc: the `.id(..)` is what makes SetValue/Increment/Decrement
    // routable; anonymous SpinButtons drop AT-routed edits silently.
    .id(iced::advanced::widget::Id::new(
        "terminal-emulator/settings/scrollback",
    ))
    // preferences_row `.control` cannot rename an opaque element; the
    // caller owns the accessible name. Match the row title so screen
    // readers hear the same string on the SpinButton, without a visible
    // caption repeating the title (and its range) above the control.
    .aria_label(model::SCROLLBACK_BOUNDS_LABEL.as_str())
    .into_element();
    // The stepper shows the value, so the row carries no "Currently" line.
    let scrollback_row = enz_preferences_row(model::SCROLLBACK_BOUNDS_LABEL.as_str())
        .control(scrollback_stepper)
        .into_element();

    // The "Darker" / "Dark" / "Black" buttons are GONE. They wrote
    // `Settings::background`, which `grid_view::terminal_background` only
    // consulted when the active scheme did not force a canvas — true of
    // exactly one scheme, the low-contrast `Default` mapping this issue
    // deleted. A control presented as live and deliberately inert is worse
    // than a missing control, so the canvas is now the scheme's, full stop.
    // The absence gate is
    // `the_three_background_presets_are_absent_from_the_settings_dialog`.
    let appearance_group =
        enz_preferences_group(column![font_size_row, scrollback_row].spacing(sp.xs()))
            .title(crate::i18n::label(APPEARANCE_TITLE))
            .spacing_scale(sp)
            .typography_scale(enz.typography);

    // There is no "Done" button: the dismiss is the surface's circular X
    // (accessible name "Close"), and a labelled "Done" would both duplicate it
    // and imply a confirm/apply step settings does not have.
    //
    // Reset appearance lives in the preset's pinned footer, not inside the pane
    // body.
    //
    // Stage 2 had to put them in the body — the preset owned header + one
    // scrolling pane and had no other slot — so they scrolled away with the
    // content: to reach the reset commands the user had to scroll past
    // every setting first, and they were LAST in a pane long enough
    // to need scrolling. `.actions([..])` lays them out below the scroll
    // viewport as a sibling of it, so they hold their position at every
    // scroll offset.
    //
    // It belongs there and per-setting controls do not: it is a verb over
    // the whole appearance domain, carries no value, and must stay
    // reachable while the user is anywhere in the pane. The
    // framework type enforces that — `SettingsAction` has a label and an
    // activation and no value channel, so a toggle is not expressible in
    // this slot.
    //
    // `settings_action` is lazy (the message factory runs on press), which
    // also drops the eagerly-constructed `Message` the old row built on
    // every view pass.
    let appearance_body = column![main_group, appearance_group]
        .spacing(sp.space7)
        .width(Length::Fill);
    // Settings load and save failures are bottom-right notices
    // (`App::preference_notices`), not part of the pane.

    let shortcuts_body = enz_editable_shortcuts(
        &app.shortcuts,
        app.shortcut_armed.as_deref(),
        Message::EditShortcut,
    )
    .feedback(
        app.shortcut_feedback
            .as_ref()
            .map(|(action, message)| (action.as_str(), message.as_str())),
    )
    .into_element();

    // ADOPT the framework `enz_settings_dialog` preset.
    //
    // This is one flat pane (two `enz_preferences_group` stacks + a reset
    // footer), no page navigation — `SettingsNav::None` (a single page). The
    // preset owns the modal surface, the per-rung geometry from the live
    // window size, and the circular-X dismiss; `into_surface(..)` yields the
    // surface content that drops into the app's existing `enz_modal_host`
    // slot (wired in `fn view`, funnelling scrim/Escape/Tab through
    // `SettingsModalEvent`). The X's `on_done` routes to `CloseSettings`,
    // the same message the reducer already handles.
    //
    // `DialogSize::Compact` is 460 px at the wide rung, exactly this dialog's
    // tuned `dialog_width_md`. `.headerless(true)` suppresses the preset's own
    // header, which would publish SETTINGS_TITLE a third time (the first
    // `enz_preferences_group` is also titled "Terminal settings", identical to
    // the dialog title). The string reaches the AT twice, the modal host's
    // Role::Dialog label and the group's Role::Group heading. Gated by
    // `the_settings_dialog_title_is_published_exactly_twice`.
    // The same builder (same two pages, same navigation, same controls) is
    // hosted as the framework's bottom-attached `into_sheet_surface` when the
    // window is narrow or squat, and stays the centred Small dialog otherwise.
    // The choice is asked of the framework here and again in
    // [`Self::settings_placement`] for the host attachment and exit motion;
    // both read `settings_window_size()` so one window resolves one decision.
    // Reverting `.presentation` or reverting the branch to a bare
    // `.into_surface(..)` reddens the census and the app gates below.
    let window = window_size(app);
    let builder = enz_settings_dialog::<Message>(true, window)
        // "Settings", as every other app titles the dialog.
        .title(crate::i18n::label("Settings"))
        .size(enzui_widgets::settings_dialog::DialogSize::Small)
        .presentation(EnzSettingsPresentation::Adaptive)
        .page(
            SettingsPage::new("appearance", crate::i18n::label("Appearance")).icon("settings"),
            appearance_body,
        )
        .page(shortcuts_settings_page(), shortcuts_body)
        .selected(app.settings_page.clone())
        .on_select(Message::SelectSettingsPage)
        .stacked_navigation(app.settings_drilled_in, Message::SettingsBack)
        .actions([
            settings_action(
                SETTINGS_RESET_APPEARANCE_ID,
                crate::i18n::label("Reset appearance"),
                || Message::ResetAppearance,
            ),
            editable_shortcuts_reset_all_action(|| {
                Message::EditShortcut(EditableShortcutsEvent::ResetAll)
            }),
        ])
        .page_id_prefix(SETTINGS_ID_PREFIX);
    let on_done: Box<dyn Fn() -> Message> = Box::new(|| Message::CloseSettings);
    match builder.resolved_presentation() {
        EnzSettingsPresentation::Sheet => builder.into_sheet_surface(Some(on_done)),
        EnzSettingsPresentation::Dialog | EnzSettingsPresentation::Adaptive => {
            builder.into_surface(Some(on_done))
        }
    }
}

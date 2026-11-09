//! Terminal reflow and subscription methods.

use super::*;

impl App {
    /// Reflow the active PTYs when `view` has published a pane-grid allocation
    /// that differs from the one they were last sized for, so a banner
    /// appearing or clearing above the grid resizes the terminal within a poll.
    pub(crate) fn reflow_if_grid_area_changed(&mut self) {
        let area = self.grid_area.get();
        if area != self.resized_grid_area {
            self.resized_grid_area = area;
            self.resize_active_pty();
        }
    }

    // ----- subscription ----------------------------------------------------

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        // PTY output is an EVENT, not something to discover by asking 30
        // times a second.
        //
        // This used to be `iced::time::every(33ms).map(|_| Message::Poll)`. iced
        // repaints whenever `update` runs, so that timer meant an idle terminal
        // — a static shell prompt, nothing happening — still ran ~30
        // update+layout+shape+render cycles a second forever. Measured on the
        // pre-fix binary: 100.4% of one core (software renderer) and 15.25
        // committed frames/second on the Wayland wire, with the profile
        // dominated by `cosmic_text`/`fontdb` glyph shaping — re-shaping text
        // that had not changed.
        //
        // `pty::wake_stream` yields only when a reader thread has actually fed
        // bytes into a `Screen` (see `pty::WakeBus`). Idle therefore costs zero
        // wakes and zero repaints, and a busy PTY is no longer capped at 30 Hz:
        // output redraws as fast as it arrives, coalesced per burst.
        //
        // `has_initial_resize` is deliberately NOT consulted any more. It
        // existed to stop the 33 ms timer forcing a paint before the
        // compositor's configure event landed, which made the first frame use
        // winit's stale 1024x768 default. With no timer there is no such
        // premature paint to suppress: the first frame is drawn from the
        // resize event itself, and a PTY wake that beats it still lays out
        // against `content_size`, which `sidebar_force` pins to Hidden while it
        // is zero. The field stays because `resize_active_pty` uses it to avoid
        // sizing a PTY from a size nobody has reported yet.
        let poll = Subscription::run(|| pty::wake_stream(pty::wake_bus())).map(|()| Message::Poll);

        let events = iced::event::listen_with(|event, _status, win| match event {
            iced::Event::Window(event) => window_focus_message(win, event),
            _ => None,
        });
        let inputs = self.input_subscription();
        let chrome = enzui_widgets::app_shell::ShellWindowState::events().map(Message::WindowState);
        // Dedicated resize subscription — fires on the initial configure too, so
        // the window size is known even when the compositor creates the window
        // at the target rung size with no follow-up Resized event.
        //
        // The id is kept: `resize_events()` reports every window's resizes, and
        // dropping the id would let a detached terminal being dragged wider
        // drive the main window's responsive rungs. The reducer files the size
        // against the window it belongs to.
        let resizes =
            iced::window::resize_events().map(|(id, size)| Message::WindowResized(id, size));
        // A request is interceptable and a destruction is not: this is the
        // app's decision point, and it is the only one.
        let close_requests = iced::window::close_requests().map(Message::CloseRequested);
        // A daemon does not stop when its last window closes. Without this
        // subscription the user's last close would leave the process running
        // with no surfaces, holding its PTYs. It carries windows that are
        // already gone: this app's own committed closes, and destructions no
        // request could intercept.
        let closes = iced::window::close_events().map(Message::WindowClosed);
        let themes = self
            .themes
            .subscription()
            .map(|event| Message::Theme(Box::new(event)));
        let uri_portal = self
            .uri_portal
            .completion_subscription()
            .map(Message::UriPortalWake);
        let toast_expiry =
            self.toast_expiry
                .armed()
                .map_or_else(Subscription::none, |(deadline, wait)| {
                    // The interval subscription is armed only until its FIRST
                    // message: that tick expires the sole visible toast and the
                    // reducer removes this recipe. `.with(deadline)` makes a
                    // replacement a distinct recipe while unrelated PTY updates
                    // keep the original schedule. No frame stream or short-period
                    // polling loop runs during the dwell.
                    iced::time::every(wait)
                        .with(deadline)
                        .map(|(_deadline, now)| Message::Toast(ToastEvent::Tick(now)))
                });
        let subscriptions = vec![
            self.shell_activity.subscription(),
            poll,
            inputs,
            chrome,
            events,
            resizes,
            close_requests,
            closes,
            themes,
            uri_portal,
            toast_expiry,
            self.openssh_paint_subscription(),
            self.credential_mutation_subscription(),
            self.close_settle_subscription(),
            self.teardown_exit_subscription(),
            #[cfg(feature = "center")]
            self.center_subscription(),
        ];
        #[cfg(feature = "native-invalidation-acceptance")]
        let subscriptions = if self.native_acceptance.is_some() {
            subscriptions
                .into_iter()
                .chain(std::iter::once(
                    iced::time::every(Duration::from_millis(25)).filter_map(|_| {
                        enzui_core::app_runtime::native_invalidation::control_changed()
                            .then_some(Message::NativeAcceptancePoll)
                    }),
                ))
                .collect()
        } else {
            subscriptions
        };
        Subscription::batch(subscriptions)
    }
}

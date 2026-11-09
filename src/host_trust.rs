//! Recovery for a saved-secret connection refused for an unconfirmed host key.
//!
//! `sshpass` reports [`SSHPASS_HOST_KEY_UNKNOWN`] before delivering anything,
//! so the pane keeps a profile whose secret was never sent. The offer here
//! re-launches the same endpoint snapshot interactively without that secret and
//! leaves the fingerprint decision to OpenSSH and the user: nothing in this
//! module accepts a key, writes `known_hosts`, weakens
//! `StrictHostKeyChecking`, or sends the secret again.

use crate::model::{SshAuthMode, SshHostRecord};
use crate::operation_origin::OperationOrigin;
use crate::pane_endpoint::PaneEndpoint;
use crate::{App, Message, model, pty};
use enzui_core::cancellation::CancelCause;
use iced::{Element, Length, Task};

/// The `sshpass` status for an unknown host public key, which it reports
/// without confirming that key.
///
/// `sshpass` forwards `ssh`'s own status once `ssh` runs to completion, so this
/// value means a refused host key only for a child launched with a secret
/// channel.
pub(crate) const SSHPASS_HOST_KEY_UNKNOWN: i32 = 6;

pub(crate) const HOST_KEY_UNCONFIRMED: &str = "Host key not confirmed";

/// One pane's owed host-trust confirmation.
///
/// Shown and acted on only while [`App::host_trust_offer_is_current`] holds, so
/// a stale offer can never confirm trust for a replacement endpoint.
pub(crate) struct HostTrustOffer {
    pub(crate) origin: OperationOrigin,
    endpoint: PaneEndpoint,
    profile: String,
    cwd: String,
}

impl Drop for HostTrustOffer {
    fn drop(&mut self) {
        self.origin.generation.retire(CancelCause::Disconnected);
    }
}

// Carries no secret and no copyable shell command.
fn trust_body(host: &SshHostRecord, actionable: bool) -> String {
    let endpoint = if host.username.is_empty() {
        format!("{}:{}", host.destination, host.port)
    } else {
        format!("{}@{}:{}", host.username, host.destination, host.port)
    };
    let mut body = crate::i18n::format(
        crate::i18n::localizer(),
        "The saved secret for {} was never sent: {endpoint} is not a confirmed host in known_hosts. Confirm this host's fingerprint before connecting this profile again; nothing was accepted on your behalf.",
        &[("arg0", &(host.name)), ("endpoint", &(endpoint))],
    );
    if actionable {
        body.push_str(
            " Confirm host key opens the same endpoint interactively without sending your saved secret, so OpenSSH shows you the fingerprint and records your own answer in known_hosts.",
        );
    }
    body
}

impl App {
    /// Record the host-trust recovery a refused secret delivery earns.
    ///
    /// Only [`SSHPASS_HOST_KEY_UNKNOWN`] from a secret-delivering child on a
    /// password or passphrase profile that still resolves earns one; every
    /// other exit leaves the pane untouched.
    pub(crate) fn note_host_trust_exit(
        &mut self,
        session_id: &str,
        pane_id: &str,
        exit: &pty::ChildExit,
        secret_delivery: bool,
    ) {
        if !secret_delivery || exit.code != Some(SSHPASS_HOST_KEY_UNKNOWN) {
            return;
        }
        let Some((endpoint, cwd)) = self
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .and_then(|session| session.panes.iter().find(|pane| pane.id == pane_id))
            .map(|pane| (pane.endpoint.clone(), pane.cwd.clone()))
        else {
            return;
        };
        let Some(profile) = secret_profile(&endpoint).map(|host| host.id.clone()) else {
            return;
        };
        if endpoint
            .resolve(&self.ssh_hosts, &self.openssh_hosts)
            .is_err()
        {
            return;
        }
        let Some(origin) = self.operation_origin_for(session_id, pane_id) else {
            return;
        };
        self.host_trust
            .retain(|offer| offer.origin.pane != origin.pane);
        self.host_trust.push(HostTrustOffer {
            origin,
            endpoint,
            profile,
            cwd,
        });
    }

    /// Whether this offer may still be shown and acted on.
    ///
    /// Excludes the pane's teardown debt on purpose: the refusal is recorded as
    /// the exited child is retired, so the pane always owes one for a moment.
    /// [`App::confirm_host_key`] checks it separately.
    pub(crate) fn host_trust_offer_is_current(&self, offer: &HostTrustOffer) -> bool {
        self.operation_origin_is_live(&offer.origin)
            && !self.ptys.contains_key(&offer.origin.pane)
            && secret_profile(&offer.endpoint).is_some_and(|host| host.id == offer.profile)
            && self
                .sessions
                .iter()
                .find(|session| session.id == offer.origin.session)
                .and_then(|session| {
                    session
                        .panes
                        .iter()
                        .find(|pane| pane.id == offer.origin.pane)
                })
                .is_some_and(|pane| pane.endpoint == offer.endpoint)
            && offer
                .endpoint
                .resolve(&self.ssh_hosts, &self.openssh_hosts)
                .is_ok()
    }

    /// Drop every offer that is no longer current, retiring its generation so a
    /// queued confirmation cannot still apply.
    pub(crate) fn reconcile_host_trust(&mut self) {
        let mut offers = std::mem::take(&mut self.host_trust);
        offers.retain(|offer| self.host_trust_offer_is_current(offer));
        self.host_trust = offers;
    }

    /// Start the interactive, secret-free session that confirms the host key for
    /// the pane this offer belongs to.
    ///
    /// Passing no password takes the direct `ssh` branch: no `sshpass`, no
    /// secret channel, and the unchanged `StrictHostKeyChecking=ask` prompt on
    /// the pane's own pty. A spent offer is consumed whether or not the child
    /// starts, while a pane still owing teardown keeps its offer and starts
    /// nothing beside that child.
    pub(crate) fn confirm_host_key(&mut self, origin: &OperationOrigin) -> Task<Message> {
        let Some(index) = self
            .host_trust
            .iter()
            .position(|offer| offer.origin == *origin)
        else {
            return Task::none();
        };
        if self.pane_has_teardown_debt(&origin.pane) {
            // A dead pty produces no further wake, so a settled teardown has to
            // be collected on this press.
            self.poll_teardown_debts();
        }
        if !self.host_trust_offer_is_current(&self.host_trust[index])
            || self.pane_has_teardown_debt(&origin.pane)
            || self.pane_spawn_pending(&origin.pane)
        {
            return Task::none();
        }
        let offer = self.host_trust.remove(index);
        self.start_endpoint_spawn(&offer.origin.pane, &offer.endpoint, &offer.cwd, None)
    }

    /// Prepend this pane's owed host-trust prose to its surface.
    ///
    /// The action appears only while the reducer can accept it: the startup gate
    /// refuses unrelated messages, so until it clears the banner carries
    /// instructions alone rather than an inert button.
    pub(crate) fn host_trust_notice<'a>(
        &self,
        pane: &model::Pane,
        content: Element<'a, Message>,
    ) -> Element<'a, Message> {
        use enzui_widgets::{banner::BannerTone, enz_banner};
        use iced::widget::column;

        let Some((offer, host)) = self
            .host_trust
            .iter()
            .find(|offer| offer.origin.pane == pane.id)
            .filter(|offer| self.host_trust_offer_is_current(offer))
            .and_then(|offer| secret_profile(&offer.endpoint).map(|host| (offer, host)))
        else {
            return content;
        };
        let actionable = self.startup.is_none();
        let mut banner = enz_banner(crate::i18n::label(HOST_KEY_UNCONFIRMED))
            .body(trust_body(host, actionable))
            .tone(BannerTone::Danger)
            .dismissible(false);
        if actionable {
            banner = banner
                .action_text(crate::i18n::label("Confirm host key"))
                .on_action(Message::ConfirmHostKey(offer.origin.clone()));
        }
        column![banner.into_element(), content]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

fn secret_profile(endpoint: &PaneEndpoint) -> Option<&SshHostRecord> {
    match endpoint {
        PaneEndpoint::Profile(host)
            if matches!(
                host.auth,
                SshAuthMode::Password | SshAuthMode::KeyFile { .. }
            ) =>
        {
            Some(host)
        }
        _ => None,
    }
}

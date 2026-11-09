//! Terminal SSH host-list method.

use super::*;
use iced::widget::column;

impl App {
    /// The host list view inside the SSH manager dialog.
    pub(crate) fn ssh_host_list_view(
        &self,
        sp: &enzui_theme::SpacingScale,
        _enz: &enzui_theme::EnzTheme,
    ) -> Element<'_, Message> {
        use enzui_widgets::enz_text::{TextSize, TextTone, enz_text};

        let mut content = column![
            enz_text(crate::i18n::label(SSH_MANAGER_TITLE))
                .size(TextSize::Headline)
                .tone(TextTone::Default),
            enz_section_header::<Message>(crate::i18n::label("Saved connections")).into_element(),
        ]
        .spacing(sp.sm())
        .width(Length::Fill);

        if self.ssh_hosts.is_empty() {
            // Empty state, routed through the shipped `enz_status_page` (the
            // same centred empty-state hero other apps use) rather than a lone
            // muted `enz_text`, which has no icon badge, title hierarchy,
            // centred layout or min-height floor. The `Inline` variant keeps
            // the badge compact for the short dialog width.
            content = content.push(
                enz_status_page(crate::i18n::label("No saved hosts yet."))
                    .description(crate::i18n::label("Add a connection to save an SSH host."))
                    .icon("server")
                    .variant(StatusPageVariant::Inline)
                    .into_element(),
            );
        } else {
            // Host list.
            for (i, host) in self.ssh_hosts.iter().enumerate() {
                let agent_status = if matches!(host.auth, SshAuthMode::Agent) {
                    if model::ssh_agent_available() {
                        crate::i18n::label(" · Agent is ready")
                    } else {
                        crate::i18n::label(" · Agent unavailable")
                    }
                } else {
                    ""
                };
                let auth_label = match &host.auth {
                    SshAuthMode::Password => crate::i18n::label("Password"),
                    SshAuthMode::KeyFile { .. } => crate::i18n::label("Key file"),
                    SshAuthMode::Agent => crate::i18n::label("Agent"),
                };
                // The saved-secret badge is answered by
                // `CredentialStore::contains` — the cheap, NON-unlocking
                // existence query. Painting a row must never fire a `get`,
                // which on a real locked keyring pops an unlock prompt; a
                // backend that cannot answer cheaply says `Unknown`, and this
                // renders that as its own words rather than as "nothing
                // saved".
                let secret_badge = Self::secret_slot_for(&host.auth)
                    .map(|slot| {
                        let presence = self.credentials.contains(&slot.key(&host.id));
                        format!(" · {}", ssh_credentials::presence_badge(presence, slot))
                    })
                    .unwrap_or_default();
                let subtitle = format!(
                    "{} · {}{}{}",
                    host.destination, auth_label, secret_badge, agent_status
                );
                let mut host_details = column![
                    enz_text(host.name.clone())
                        .size(TextSize::Footnote)
                        .tone(TextTone::Default),
                    enz_text(subtitle)
                        .size(TextSize::Caption)
                        .tone(TextTone::Muted),
                ];
                if !host.notes.trim().is_empty() {
                    host_details = host_details.push(
                        enz_text(crate::i18n::format(
                            crate::i18n::localizer(),
                            "Notes: {}",
                            &[("arg0", &(host.notes.trim()))],
                        ))
                        .size(TextSize::Caption)
                        .tone(TextTone::Muted),
                    );
                }
                // Name-over-subtitle/notes typographic ramp inside one host
                // row: a 2px hairline that must not read as a gap between
                // sibling controls. `SpaceToken::Xxs` (= space1 = 2px) is
                // exactly that rung, so this tracks the ladder instead of a
                // literal.
                host_details = host_details
                    .spacing(enzui_theme::SpaceToken::Xxs.resolve(sp))
                    .width(Length::Fill);

                // The whole row connects: it highlights under the pointer like
                // any list row and ends in an arrow, so there is no Connect
                // button to aim for. Edit and Delete are small icon buttons
                // beside the arrow. Each control is named after the row's
                // host, disambiguated when two saved hosts share a name, so a
                // screen reader can route to one.
                let at_name = saved_host_at_name(&self.ssh_hosts, i);
                let named = |text: &str| {
                    crate::i18n::format(crate::i18n::localizer(), text, &[("at_name", &(at_name))])
                };
                let edit = named("Edit SSH host {at_name}");
                let delete = named("Delete SSH host {at_name}");
                let actions = row![
                    enz_toolbar_icon("pencil", Message::OpenSshEditForm(i))
                        .id(derive_id(SAVED_HOST_EDIT_ID_PREFIX, &host.id))
                        .aria_label(edit.clone())
                        .tooltip(edit)
                        .into_element(),
                    enz_toolbar_icon("trash-2", Message::DeleteSshHost(i))
                        .id(derive_id(SAVED_HOST_DELETE_ID_PREFIX, &host.id))
                        .aria_label(delete.clone())
                        .tooltip(delete)
                        .into_element(),
                    connect_arrow(),
                ]
                .spacing(enzui_theme::SpaceToken::Xxs.resolve(sp))
                .align_y(Alignment::Center);
                content = content.push(
                    enz_list_row_surface(host_details, Message::ConnectSshHost(i))
                        .label(named("Connect to SSH host {at_name}"))
                        // Identity comes from the record's OWN stable
                        // `host.id` (`ssh-host-N`, minted once at save and
                        // preserved across edits), never from the renameable,
                        // non-unique display name the default derivation would
                        // use: a routed focus request must reach the row it
                        // named even after a rename or a reorder.
                        .id(derive_id(SAVED_HOST_ROW_ID_PREFIX, &host.id))
                        .trailing(actions)
                        .width(Length::Fill)
                        .content_height(true),
                );
            }
        }

        content = content.push(
            enz_section_header::<Message>(crate::i18n::label("OpenSSH config")).into_element(),
        );
        content = content.push(
            enz_button(
                crate::i18n::label("Reload OpenSSH config"),
                Message::ReloadOpenSshConfig,
            )
            .into_element(),
        );
        if matches!(
            self.openssh_discovery.phase,
            ssh_discovery::Phase::AwaitingFrame | ssh_discovery::Phase::Loading
        ) {
            content = content.push(
                enz_text(crate::i18n::label("Loading OpenSSH hosts…"))
                    .size(TextSize::Caption)
                    .tone(TextTone::Muted),
            );
        } else if self.openssh_discovery.phase == ssh_discovery::Phase::Unavailable
            && self.openssh_load_error.is_none()
        {
            content = content.push(
                enz_banner(crate::i18n::label(
                    "OpenSSH discovery is unavailable. Reload to retry.",
                ))
                .tone(BannerTone::Danger)
                .dismissible(false)
                .into_element(),
            );
        } else if self.openssh_hosts.is_empty() && self.openssh_load_error.is_none() {
            content = content.push(
                enz_text(crate::i18n::label("No named OpenSSH hosts found."))
                    .size(TextSize::Caption)
                    .tone(TextTone::Muted),
            );
        } else {
            let mut alias_counts = HashMap::new();
            for host in &self.openssh_hosts {
                *alias_counts.entry(host.alias.as_str()).or_insert(0_usize) += 1;
            }
            for (index, host) in self.openssh_hosts.iter().enumerate() {
                let destination = if host.user.is_empty() {
                    format!("{}:{}", host.hostname, host.port)
                } else {
                    format!("{}@{}:{}", host.user, host.hostname, host.port)
                };
                let at_alias = if alias_counts.get(host.alias.as_str()) == Some(&1) {
                    host.alias.clone()
                } else {
                    openssh_host_at_name(&self.openssh_hosts, index)
                };
                let details = column![
                    enz_text(host.alias.clone())
                        .size(TextSize::Footnote)
                        .tone(TextTone::Default),
                    enz_text(destination)
                        .size(TextSize::Caption)
                        .tone(TextTone::Muted),
                ]
                .spacing(enzui_theme::SpaceToken::Xxs.resolve(sp))
                .width(Length::Fill);
                content = content.push(
                    enz_list_row_surface(
                        details,
                        Message::ConnectOpenSshHost(index, self.openssh_discovery.generation()),
                    )
                    .label(crate::i18n::format(
                        crate::i18n::localizer(),
                        "Connect to OpenSSH host {at_alias}",
                        &[("at_alias", &(at_alias))],
                    ))
                    .id(derive_id(OPENSSH_HOST_ROW_ID_PREFIX, &at_alias))
                    .trailing(connect_arrow())
                    .width(Length::Fill)
                    .content_height(true),
                );
            }
        }
        if let Some(error) = &self.openssh_load_error {
            content = content.push(
                enz_banner(crate::i18n::text(error))
                    .tone(BannerTone::Danger)
                    .dismissible(false)
                    .into_element(),
            );
        }
        // Settings load, profile save and keyring outcomes are bottom-right
        // notices (`App::preference_notices`); only the discovery state above
        // stays beside its Reload button.
        // Cancellation before provider entry prevents the mutation; after
        // entry, the exact key remains pending until provider reconciliation.
        if self.credential_op_in_flight() {
            let action = if self.credential_mutation_waiting() {
                enz_button(
                    crate::i18n::label("Retry keyring"),
                    Message::RetryCredentialMutations,
                )
            } else {
                enz_button(
                    crate::i18n::label("Cancel keyring request"),
                    Message::CancelSshCredentialOps,
                )
            };
            content = content.push(
                row![
                    enz_text(crate::i18n::label("Waiting for the system keyring\u{2026}"))
                        .size(TextSize::Caption)
                        .tone(TextTone::Muted),
                    action.kind(ButtonKind::Ghost).into_element(),
                ]
                .spacing(sp.xs())
                .align_y(Alignment::Center)
                .width(Length::Fill),
            );
        }

        // The surface owns the manager's only dismiss affordance. This row is
        // a manager command, not a second Close route.
        let actions = row![
            enz_button(
                crate::i18n::label("Add connection"),
                Message::OpenSshAddForm
            )
            .kind(ButtonKind::Ghost)
            .into_element(),
        ]
        .spacing(sp.sm())
        .width(Length::Fill);

        column![content, actions]
            .spacing(sp.md())
            .width(Length::Fill)
            .into()
    }
}

/// The arrow a host row ends in: the row itself connects, so this is a
/// marker, not a control.
fn connect_arrow<'a>() -> Element<'a, Message> {
    enzui_widgets::enz_icon("arrow-right")
        .size(16.0)
        .tint_theme_tone(enzui_widgets::icon::IconTone::Muted)
        .into_element()
}

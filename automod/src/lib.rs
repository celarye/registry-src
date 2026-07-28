use std::{fmt::Write, time::UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use twilight_http::{
    Client,
    request::{AuditLogReason, TryIntoRequest},
};
use twilight_model::{
    channel::{
        Channel,
        message::{
            Embed,
            embed::{EmbedAuthor, EmbedFooter},
        },
    },
    gateway::payload::incoming::MessageCreate,
    id::Id,
    util::Timestamp,
};

use crate::bindgen::{
    exports::wpbs::plugin::{
        core_export_functions::Guest as CoreGuest, discord_export_functions::Guest as DiscordGuest,
        job_scheduler_export_functions::Guest as JobSchedulerGuest,
    },
    wpbs::plugin::{
        core_import_functions::{get_state, register, set_state},
        core_import_types::{Registrations, ServicesRegistrations},
        core_types::PluginError,
        discord_export_types::{DiscordEvents, DiscordRegistrationsResultApplicationCommands},
        discord_import_functions::discord_request,
        discord_import_types::{Body, DiscordEventKinds, DiscordRegistrations, DiscordRequests},
    },
};

#[allow(clippy::same_length_and_capacity)]
mod bindgen {
    use crate::Plugin;

    wit_bindgen::generate!({ path: "../wit" });

    export!(Plugin);
}

struct Plugin {}

#[derive(Default, Deserialize, Serialize)]
struct PluginSettings {
    automod_channel_id: u64,
    #[serde(default = "PluginSettings::stack_time_outs_default")]
    stack_time_outs: bool,
    #[serde(default)]
    bypass: PluginSettingsBypass,
    #[serde(default)]
    validations: PluginSettingsValidations,
}

impl PluginSettings {
    fn stack_time_outs_default() -> bool {
        true
    }
}

#[derive(Default, Deserialize, Serialize)]
struct PluginSettingsBypass {
    #[serde(default)]
    users: Vec<u64>,
    #[serde(default)]
    roles: Vec<u64>,
}

#[derive(Default, Deserialize, Serialize)]
struct PluginSettingsValidations {
    #[serde(default)]
    attachment_spam: PluginSettingsAttachmentSpam,
}

#[derive(Default, Deserialize, Serialize)]
struct PluginSettingsAttachmentSpam {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "PluginSettingsAttachmentSpam::count_default")]
    count: usize,
    #[serde(default)]
    actions: Actions,
}

impl PluginSettingsAttachmentSpam {
    fn count_default() -> usize {
        4
    }
}

#[derive(Default, Deserialize, Serialize)]
struct Actions {
    #[serde(default = "Actions::report_default")]
    report: bool,
    #[serde(default = "Actions::message_default")]
    message: Option<ActionsMessage>,
    #[serde(default = "Actions::user_default")]
    user: Option<ActionsUser>,
}

impl Actions {
    fn report_default() -> bool {
        true
    }

    #[allow(clippy::unnecessary_wraps)]
    fn message_default() -> Option<ActionsMessage> {
        Some(ActionsMessage::default())
    }

    #[allow(clippy::unnecessary_wraps)]
    fn user_default() -> Option<ActionsUser> {
        Some(ActionsUser::default())
    }
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
enum ActionsMessage {
    #[default]
    Delete,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
enum ActionsUser {
    Ban,
    #[serde(rename = "time_out")]
    TimeOut(u64),
}

impl Default for ActionsUser {
    fn default() -> Self {
        Self::TimeOut(60)
    }
}

struct TakeAction {
    report: Option<String>,
    message: Option<ActionsMessage>,
    user: Option<ActionsUser>,
}

impl CoreGuest for Plugin {
    fn initialization(settings: String) -> Result<(), PluginError> {
        let settings = match sonic_rs::from_str::<PluginSettings>(&settings) {
            Ok(settings) => settings,
            Err(err) => {
                return Err(format!(
                    "The provided settings were of the incorrect structure: {err}"
                ));
            }
        };

        let get_channel_response = match discord_request(&DiscordRequests::GetChannel(
            settings.automod_channel_id,
        )) {
            Ok(get_channel_response) => get_channel_response,
            Err(err) => {
                return Err(format!(
                    "An error occured while trying to get information of the automod channel: {err}"
                ));
            }
        };

        if let Err(err) = sonic_rs::from_str::<Channel>(get_channel_response.as_ref().unwrap()) {
            return Err(format!(
                "An error occured while deserializing the get channel response from Discord: {err}",
            ));
        }

        let registrations_result = register(&Registrations {
            core: None,
            services: Some(ServicesRegistrations {
                job_scheduler: None,
                discord: Some(DiscordRegistrations {
                    events: Some(vec![DiscordEventKinds::MessageCreate]),
                    interactions: None,
                }),
            }),
        });

        match registrations_result
            .services
            .as_ref()
            .unwrap()
            .discord
            .as_ref()
            .unwrap()
        {
            Ok(discord_registrations_result) => {
                for discord_event_registration_result in
                    discord_registrations_result.events.as_ref().unwrap()
                {
                    if let Err(err) = discord_event_registration_result.1.as_ref() {
                        return Err(format!(
                            "An error occured while making a Discord event registration: {err}",
                        ));
                    }
                }
            }
            Err(err) => {
                return Err(format!(
                    "An error occured while making a Discord service registration: {err}",
                ));
            }
        }

        if let Err(err) = set_state("settings", &sonic_rs::to_vec(&settings).unwrap()) {
            return Err(format!("An error while storing the settings: {err}"));
        }

        Ok(())
    }

    fn dependency_function(_function: String, _params: Vec<u8>) -> Result<Vec<u8>, String> {
        unimplemented!();
    }

    fn shutdown() -> Result<(), String> {
        Ok(())
    }
}

impl JobSchedulerGuest for Plugin {
    fn scheduled_job(_job_id: String) -> Result<(), PluginError> {
        unimplemented!()
    }
}

impl DiscordGuest for Plugin {
    fn discord_application_commands(
        _registrations_result: DiscordRegistrationsResultApplicationCommands,
    ) {
        unimplemented!()
    }

    fn discord_event(event: DiscordEvents) -> Result<(), PluginError> {
        match event {
            DiscordEvents::MessageCreate(message_create_bytes) => {
                match sonic_rs::from_str::<Box<MessageCreate>>(&message_create_bytes) {
                    Ok(message_create) => Self::validate_message(&message_create),
                    Err(err) => Err(err.to_string()),
                }
            }
            _ => unimplemented!(),
        }
    }
}

impl Plugin {
    fn validate_message(message_create: &MessageCreate) -> Result<(), PluginError> {
        let settings = match get_state("settings") {
            Ok(settings) => {
                sonic_rs::from_slice::<PluginSettings>(settings.as_ref().unwrap()).unwrap()
            }
            Err(err) => {
                return Err(format!("An error while storing the settings: {err}"));
            }
        };

        if Self::bypass(&settings, message_create) {
            return Ok(());
        }

        let mut take_action = TakeAction {
            report: None,
            message: None,
            user: None,
        };

        if settings.validations.attachment_spam.enabled
            && let Some(new_take_action) = Self::attachment_spam(&settings, message_create)
        {
            Self::update_take_action(&mut take_action, new_take_action);
        }

        Self::take_action(&settings, &take_action, message_create)?;

        Ok(())
    }

    fn bypass(settings: &PluginSettings, message_create: &MessageCreate) -> bool {
        if settings
            .bypass
            .users
            .contains(&message_create.author.id.get())
        {
            return true;
        }

        if let Some(member) = &message_create.member {
            for member_role in &member.roles {
                if settings.bypass.roles.contains(&member_role.get()) {
                    return true;
                }
            }
        }

        false
    }

    fn attachment_spam(settings: &PluginSettings, message: &MessageCreate) -> Option<TakeAction> {
        if !message.content.is_empty() {
            return None;
        }

        let attachment_count = message.attachments.len();

        if attachment_count >= settings.validations.attachment_spam.count {
            let report = if settings.validations.attachment_spam.actions.report {
                Some(format!(
                    "Attachment spam ({attachment_count}), without message content"
                ))
            } else {
                None
            };

            return Some(TakeAction {
                report,
                message: settings.validations.attachment_spam.actions.message,
                user: settings.validations.attachment_spam.actions.user,
            });
        }

        None
    }

    fn update_take_action(take_action: &mut TakeAction, new_take_action: TakeAction) {
        if let Some(new_report) = new_take_action.report {
            if let Some(report) = &mut take_action.report {
                let _ = write!(report, "\n- {new_report}");
            } else {
                take_action.report = Some(format!("- {new_report}"));
            }
        }

        // This will need an update when other message actions get introduced
        if new_take_action.message.is_some() && take_action.message.is_none() {
            take_action.message = new_take_action.message;
        }

        //if let Some(new_message_action) = new_take_action.message {
        //    if let Some(message_action) = take_action.message {
        //        match message_action {
        //            ActionsMessage::Delete => (),
        //        }
        //    } else {
        //        take_action.message = new_take_action.message;
        //    }
        //}

        if let Some(new_user_action) = new_take_action.user {
            if let Some(user_action) = take_action.user {
                match user_action {
                    ActionsUser::Ban => (),
                    ActionsUser::TimeOut(period) => match new_user_action {
                        ActionsUser::Ban => take_action.user = new_take_action.user,
                        ActionsUser::TimeOut(new_period) => {
                            take_action.user = Some(ActionsUser::TimeOut(period + new_period));
                        }
                    },
                }
            } else {
                take_action.user = new_take_action.user;
            }
        }
    }

    fn take_action(
        settings: &PluginSettings,
        take_action: &TakeAction,
        message: &MessageCreate,
    ) -> Result<(), PluginError> {
        if let Some(message_action) = take_action.message {
            match message_action {
                ActionsMessage::Delete => Self::delete_message(message)?,
            }
        }

        if let Some(user_action) = take_action.user {
            match user_action {
                ActionsUser::Ban => Self::ban_user(take_action.report.as_deref(), message)?,
                ActionsUser::TimeOut(period) => Self::time_out_user(message, period)?,
            }
        }

        if take_action.report.is_some() {
            Self::report(settings, take_action, message)?;
        }

        Ok(())
    }

    fn report(
        settings: &PluginSettings,
        take_action: &TakeAction,
        message: &MessageCreate,
    ) -> Result<(), PluginError> {
        let mut embed = Self::base_embed(message);

        embed.description = Some(format!(
            "**Reasons:**\n{}\n\n**Actions Taken:**",
            take_action.report.as_ref().unwrap()
        ));

        let embed_description = embed.description.as_mut().unwrap();

        if let Some(message_action) = take_action.message {
            match message_action {
                ActionsMessage::Delete => embed_description.push_str("\n- Message deleted"),
            }
        }

        if let Some(user_action) = take_action.user {
            match user_action {
                ActionsUser::Ban => embed_description.push_str("\n- User banned"),
                ActionsUser::TimeOut(period) => {
                    let _ = write!(embed_description, "\n- User timed out for {period} seconds");
                }
            }
        }

        embed_description.push_str("\n\n**Message:**\n");

        if message.content.is_empty() {
            embed_description.push_str("No Content");
        } else {
            embed_description.push_str(&message.content);
        }

        embed_description.push('\n');

        if message.attachments.is_empty() {
            embed_description.push_str("\nNo Attachments");
        } else {
            for attachment in &message.attachments {
                embed_description.push('\n');
                embed_description.push_str(&attachment.url);
            }
        }

        let client = Client::builder().build();

        let create_message_request = match client
            .create_message(Id::new(settings.automod_channel_id))
            .embeds(&[embed])
            .try_into_request()
        {
            Ok(create_message_request) => create_message_request,
            Err(err) => {
                return Err(format!(
                    "An error occured while creating the report create message request: {err}"
                ));
            }
        };

        discord_request(&DiscordRequests::CreateMessage((
            settings.automod_channel_id,
            Body::Json(String::from_utf8(create_message_request.body().unwrap().into()).unwrap()),
        )))?;

        Ok(())
    }

    fn delete_message(message: &MessageCreate) -> Result<(), PluginError> {
        discord_request(&DiscordRequests::DeleteMessage((
            message.channel_id.get(),
            message.id.get(),
        )))?;

        Ok(())
    }

    fn time_out_user(message: &MessageCreate, period: u64) -> Result<(), PluginError> {
        let client = Client::builder().build();

        let update_member_request = match client
            .update_guild_member(message.guild_id.unwrap(), message.author.id)
            .communication_disabled_until(Some(
                Timestamp::from_secs(
                    (UNIX_EPOCH.elapsed().unwrap_or_default().as_secs() + period)
                        .try_into()
                        .unwrap_or_default(),
                )
                .unwrap_or(Timestamp::from_secs(0).unwrap()),
            ))
            .try_into_request()
        {
            Ok(update_member_request) => update_member_request,
            Err(err) => {
                return Err(format!(
                    "An error occured while creating the update member request: {err}"
                ));
            }
        };

        discord_request(&DiscordRequests::UpdateMember((
            message.guild_id.unwrap().get(),
            message.author.id.get(),
            String::from_utf8(update_member_request.body().unwrap().into()).unwrap(),
        )))?;

        Ok(())
    }

    fn ban_user(reason: Option<&str>, message: &MessageCreate) -> Result<(), PluginError> {
        let client = Client::builder().build();

        let create_ban_request = match client
            .create_ban(message.guild_id.unwrap(), message.author.id)
            .reason(reason.unwrap_or("No reason provided"))
            .try_into_request()
        {
            Ok(create_ban_request) => create_ban_request,
            Err(err) => {
                return Err(format!(
                    "An error occured while creating the create ban request: {err}"
                ));
            }
        };

        discord_request(&DiscordRequests::CreateBan((
            message.guild_id.unwrap().get(),
            message.author.id.get(),
            String::from_utf8(create_ban_request.body().unwrap().into()).unwrap(),
        )))?;

        Ok(())
    }

    fn base_embed(message: &MessageCreate) -> Embed {
        Embed {
            author: Some(EmbedAuthor {
                icon_url: message.author.avatar.map(|avatar| {
                    format!(
                        "https://cdn.discordapp.com/avatars/{}/{}.webp",
                        message.author.id.get(),
                        avatar
                    )
                }),
                name: message.author.name.clone(),
                proxy_icon_url: None,
                url: None,
            }),
            color: Some(0x00E7_2323),
            description: None,
            fields: vec![],
            footer: Some(EmbedFooter {
                icon_url: None,
                proxy_icon_url: None,
                text: format!("ID: {}", message.author.id.get()),
            }),
            image: None,
            kind: String::from("rich"),
            provider: None,
            thumbnail: None,
            timestamp: Some(
                Timestamp::from_secs(
                    UNIX_EPOCH
                        .elapsed()
                        .unwrap_or_default()
                        .as_secs()
                        .try_into()
                        .unwrap_or_default(),
                )
                .unwrap_or(Timestamp::from_secs(0).unwrap()),
            ),
            title: Some(String::from("Automod Report")),
            url: None,
            video: None,
        }
    }
}

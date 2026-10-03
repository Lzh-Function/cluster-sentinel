//! Where notifications go.
//!
//! A provider abstraction rather than a hard-coded destination, so a
//! deployment can point at whatever it already runs (SPEC.md §112). The MVP
//! ships a generic webhook, which covers ntfy, Gotify, Slack, Discord and
//! anything else that accepts a POST.

use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

use super::{Notification, Trigger};
use crate::config::WebhookFormat;
use crate::incident::Severity;

/// Why a notification could not be delivered.
#[derive(Debug, Error)]
pub enum ProviderError {
    /// The destination could not be reached.
    #[error("通知先{destination}に接続できません。詳細　{detail}")]
    Unreachable {
        /// Where it was trying to send.
        destination: String,
        /// What went wrong.
        detail: String,
    },
    /// The destination refused the message.
    #[error("通知先{destination}がHTTP {status}を返しました。詳細　{detail}")]
    Rejected {
        /// Where it was trying to send.
        destination: String,
        /// HTTP status.
        status: u16,
        /// Body, or an extract.
        detail: String,
    },
    /// The provider could not be built.
    #[error("通知先の設定に誤りがあります。詳細　{0}")]
    Misconfigured(String),
}

impl ProviderError {
    /// Whether retrying later might work.
    pub fn is_transient(&self) -> bool {
        match self {
            ProviderError::Unreachable { .. } => true,
            ProviderError::Rejected { status, .. } => *status >= 500 || *status == 429,
            ProviderError::Misconfigured(_) => false,
        }
    }
}

/// Somewhere notifications can be sent.
#[async_trait]
pub trait NotificationProvider: Send + Sync {
    /// Provider name, used for deduplication scope and in logs.
    fn name(&self) -> &str;

    /// Deliver one notification.
    async fn send(&self, notification: &Notification) -> Result<(), ProviderError>;
}

/// Posts JSON to a URL.
#[derive(Debug, Clone)]
pub struct WebhookProvider {
    name: String,
    url: String,
    format: WebhookFormat,
    http: reqwest::Client,
}

/// Slack's limit on a `header` block, which it rejects rather than truncates.
const SLACK_HEADER_LIMIT: usize = 150;
/// Slack's limit on a `section` text block.
const SLACK_SECTION_LIMIT: usize = 3000;

/// Cut to a limit on a character boundary, marking that something was cut.
fn truncated(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

fn slack_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn plain_section(text: &str) -> serde_json::Value {
    serde_json::json!({"type": "section", "text": {"type": "plain_text", "text": text, "emoji": false}})
}

/// Pack whole paragraphs, reporting anything that cannot fit.
fn grouped_sections(parts: impl IntoIterator<Item = String>) -> (Vec<String>, bool) {
    let mut groups = Vec::new();
    let mut group = String::new();
    let mut omitted = false;
    for part in parts {
        if part.chars().count() > SLACK_SECTION_LIMIT {
            omitted = true;
            continue;
        }
        if !group.is_empty() && group.chars().count() + 2 + part.chars().count() > SLACK_SECTION_LIMIT {
            groups.push(std::mem::take(&mut group));
        }
        if !group.is_empty() {
            group.push_str("\n\n");
        }
        group.push_str(&part);
    }
    if !group.is_empty() {
        groups.push(group);
    }
    (groups, omitted)
}

fn rich_section(text: &str) -> serde_json::Value {
    serde_json::json!({"type": "rich_text_section", "elements": [{"type": "text", "text": text}]})
}

fn action_description(description: &str) -> String {
    if let Some((location, check)) = description.split_once("で、") {
        format!(
            "実行先　{location}\n{}",
            check.replace('。', "。\n").trim_end_matches('\n')
        )
    } else {
        description.replace('。', "。\n").trim_end_matches('\n').to_string()
    }
}

/// New instructions contain a description followed by a newline and commands.
/// Older persisted diagnoses can contain only a command, or only guidance.
fn action_elements(action: &str, number: Option<usize>) -> Vec<serde_json::Value> {
    let first_word = action.split_whitespace().next().unwrap_or_default();
    let is_command = matches!(
        first_word,
        "sudo"
            | "sentinel"
            | "systemctl"
            | "journalctl"
            | "scontrol"
            | "sinfo"
            | "ss"
            | "ip"
            | "ping"
            | "ssh"
            | "curl"
            | "nc"
            | "cat"
            | "ls"
            | "df"
            | "ps"
            | "pgrep"
            | "findmnt"
            | "mount"
            | "exportfs"
            | "nvidia-smi"
            | "timeout"
    );
    let (description, command) = if is_command {
        ("確認コマンド", Some(action))
    } else if let Some((description, command)) = action.split_once('\n') {
        (description, Some(command))
    } else {
        (action, None)
    };
    let description = action_description(description);
    let description = match number {
        Some(number) => format!("{number}. {description}"),
        None => description.to_string(),
    };
    let mut elements = vec![rich_section(&description)];
    if let Some(command) = command.filter(|command| !command.trim().is_empty()) {
        // Text elements are literal: quotes, backticks and Slack mention syntax
        // remain part of the command, without escaping or changing copy/paste.
        elements.push(serde_json::json!({
            "type": "rich_text_preformatted",
            "elements": [{"type": "text", "text": command}]
        }));
    }
    elements
}

/// Pack complete instructions into rich text blocks, never cutting a command.
fn grouped_actions(actions: &[String]) -> (Vec<serde_json::Value>, bool) {
    let mut groups = Vec::new();
    let mut group = Vec::new();
    let mut group_length = 0;
    let mut omitted = false;
    for (index, action) in actions.iter().enumerate() {
        let elements = action_elements(action, Some(index + 1));
        let length: usize = elements
            .iter()
            .flat_map(|element| element["elements"].as_array().into_iter().flatten())
            .filter_map(|element| element["text"].as_str())
            .map(|text| text.chars().count())
            .sum::<usize>()
            + elements.len().saturating_sub(1);
        if length > SLACK_SECTION_LIMIT {
            omitted = true;
            continue;
        }
        if !group.is_empty() && group_length + 3 + length > SLACK_SECTION_LIMIT {
            groups.push(serde_json::json!({"type": "rich_text", "elements": group}));
            group = Vec::new();
            group_length = 0;
        }
        group_length += length + usize::from(!group.is_empty()) * 3;
        if !group.is_empty() {
            // Separate numbered instructions, including those in the same block.
            group.push(rich_section("\n"));
        }
        group.extend(elements);
    }
    if !group.is_empty() {
        groups.push(serde_json::json!({"type": "rich_text", "elements": group}));
    }
    (groups, omitted)
}

impl WebhookProvider {
    /// Build a webhook provider.
    pub fn new(name: impl Into<String>, url: impl Into<String>, timeout: Duration) -> Result<Self, ProviderError> {
        let url = url.into();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ProviderError::Misconfigured(format!(
                "{url}はhttp(s)のURLではありません"
            )));
        }

        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| ProviderError::Misconfigured(e.to_string()))?;

        Ok(Self {
            name: name.into(),
            url,
            format: WebhookFormat::default(),
            http,
        })
    }

    /// Builder: shape the payload for a particular service.
    pub fn with_format(mut self, format: WebhookFormat) -> Self {
        self.format = format;
        self
    }

    /// The destination URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The whole notification as one block of readable text.
    ///
    /// Generic webhook destinations receive this alongside structured fields.
    /// Slack uses only its coloured attachment, with a notification fallback.
    fn message(notification: &Notification) -> String {
        let mut text = format!("{}\n\n{}", notification.title, notification.body);
        if !notification.recommended_actions.is_empty() {
            // The body may or may not end in a newline depending on what built
            // it, and a heading glued to the end of a sentence reads as a typo.
            if !text.ends_with("\n\n") {
                text.push_str(if text.ends_with('\n') { "\n" } else { "\n\n" });
            }
            text.push_str("確認手順\n");
            for (index, action) in notification.recommended_actions.iter().enumerate() {
                text.push_str(&format!("{}. {action}\n", index + 1));
            }
        }
        text
    }

    fn color(notification: &Notification) -> &'static str {
        match notification.trigger {
            Trigger::Resolved => "#2e7d32",
            Trigger::Recovering => "#f9a825",
            _ => match notification.severity {
                Severity::Critical => "#d32f2f",
                Severity::Warning => "#f9a825",
                Severity::Info => "#546e7a",
            },
        }
    }

    fn mentions_channel(notification: &Notification) -> bool {
        notification.severity == Severity::Critical && !notification.trigger.is_recovery()
    }

    fn summary(notification: &Notification) -> &str {
        notification
            .title
            .split_once(" / ")
            .or_else(|| notification.title.split_once(": "))
            .map(|(_, rest)| rest)
            .unwrap_or(&notification.title)
    }

    /// Slack Block Kit, with complete steps and a CLI fallback for oversized messages.
    fn slack_payload(notification: &Notification) -> serde_json::Value {
        use crate::diagnosis::investigation as guide;
        let details = guide::sentinel(&format!("incident show {}", guide::quote(&notification.incident_id)));
        let omitted = guide::step(
            "Sentinel controller",
            "表示上限のため、一部を省略しました。次のコマンドで確認手順の全文を表示してください",
            &details,
        );
        let mut blocks = Vec::new();
        if Self::mentions_channel(notification) {
            // An explicit special mention works with incoming webhooks, too.
            // Only this trusted block interprets Slack markup.
            blocks.push(serde_json::json!({
                "type": "section",
                "text": {"type": "mrkdwn", "text": "<!channel>", "verbatim": true}
            }));
        }
        blocks.push(serde_json::json!({
            "type": "header",
            "text": {"type": "plain_text", "text": truncated(notification.title.split_once(" / ").map(|(heading, _)| heading).unwrap_or(&super::heading_for(notification.trigger, notification.severity)), SLACK_HEADER_LIMIT), "emoji": false}
        }));
        if notification.body.trim().is_empty() {
            blocks.push(plain_section(&truncated(
                Self::summary(notification),
                SLACK_SECTION_LIMIT,
            )));
        }
        let (body, body_omitted) = grouped_sections(
            notification
                .body
                .split("\n\n")
                .filter(|p| !p.trim().is_empty())
                .map(str::to_string),
        );
        let mut was_omitted = body_omitted || body.len() > 10;
        blocks.extend(body.into_iter().take(10).map(|text| plain_section(&text)));
        if !notification.recommended_actions.is_empty() {
            blocks.push(plain_section("確認手順"));
            let (actions, actions_omitted) = grouped_actions(&notification.recommended_actions);
            // Reserve two blocks for the overflow notice and context.
            let available = 50usize.saturating_sub(blocks.len() + 2);
            was_omitted |= actions_omitted || actions.len() > available;
            blocks.extend(actions.into_iter().take(available));
        }
        if was_omitted {
            blocks.push(serde_json::json!({"type": "rich_text", "elements": action_elements(&omitted, None)}));
        }
        blocks.push(serde_json::json!({
            "type": "context",
            "elements": [{"type": "plain_text", "text": truncated(&format!("Cluster Sentinel / 障害ID {} / {}", notification.incident_id, notification.created_at), SLACK_SECTION_LIMIT), "emoji": false}]
        }));
        // Escape external text so names, reasons and commands cannot add mentions.
        let fallback = slack_escape(&notification.title);
        let preview = if Self::mentions_channel(notification) {
            format!("<!channel>\n{fallback}")
        } else {
            fallback
        };
        let preview = truncated(&preview, SLACK_SECTION_LIMIT);
        serde_json::json!({
            "unfurl_links": false,
            "unfurl_media": false,
            "attachments": [{
                "color": Self::color(notification),
                "fallback": preview,
                "blocks": blocks
            }]
        })
    }

    /// The JSON body sent for a notification.
    ///
    /// Generic webhooks retain structured fields for routing and recovery.
    /// Slack receives a single coloured attachment with formatted commands.
    pub fn payload_for(format: WebhookFormat, notification: &Notification) -> serde_json::Value {
        match format {
            WebhookFormat::Slack => Self::slack_payload(notification),
            WebhookFormat::Generic => Self::payload(notification),
        }
    }

    /// The generic JSON body.
    pub fn payload(notification: &Notification) -> serde_json::Value {
        let message = Self::message(notification);
        serde_json::json!({
            "source": "cluster-sentinel",
            "incident_id": notification.incident_id,
            "fingerprint": notification.fingerprint,
            "trigger": notification.trigger,
            "severity": notification.severity,
            "resolved": notification.trigger == Trigger::Resolved,
            "title": notification.title,
            "body": notification.body,
            "recommended_actions": notification.recommended_actions,
            "timestamp": notification.created_at,
            // Slack and Microsoft Teams.
            "text": message,
            // Discord.
            "content": message,
        })
    }
}

#[async_trait]
impl NotificationProvider for WebhookProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn send(&self, notification: &Notification) -> Result<(), ProviderError> {
        let response = self
            .http
            .post(&self.url)
            .json(&Self::payload_for(self.format, notification))
            .send()
            .await
            .map_err(|e| ProviderError::Unreachable {
                destination: self.url.clone(),
                detail: e.to_string(),
            })?;

        if response.status().is_success() {
            return Ok(());
        }

        let status = response.status().as_u16();
        Err(ProviderError::Rejected {
            destination: self.url.clone(),
            status,
            detail: response.text().await.unwrap_or_default().chars().take(200).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnosis::{kind, Confidence, Diagnosis};
    use crate::incident::{Incident, Severity};
    use crate::notification::Trigger;

    fn notification(trigger: Trigger) -> Notification {
        let mut incident = Incident::open("cause:fs1", Severity::Critical);
        incident.add_diagnosis(
            Diagnosis::new(kind::NFS_SERVICE_FAILURE, "storage.service_failure", Confidence::High)
                .with_summary("the export port is not answering")
                .recommending(vec!["systemctl status nfs-server".into()]),
        );
        Notification::for_incident(&incident, trigger)
    }

    fn block_text(block: &serde_json::Value) -> String {
        if let Some(text) = block["text"]["text"].as_str() {
            return text.to_string();
        }
        block["elements"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|element| element["elements"].as_array().into_iter().flatten())
            .filter_map(|element| element["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn rendered_blocks(payload: &serde_json::Value) -> String {
        payload["attachments"][0]["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(block_text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn commands(payload: &serde_json::Value) -> Vec<&str> {
        payload["attachments"][0]["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|block| block["elements"].as_array().into_iter().flatten())
            .filter(|element| element["type"] == "rich_text_preformatted")
            .map(|element| element["elements"][0]["text"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn slack_gets_a_coloured_attachment_with_blocks() {
        // Without top-level blocks, top-level text is a second visible message.
        // Keep the full notification only inside the coloured attachment.
        let payload = WebhookProvider::slack_payload(&notification(Trigger::Opened));

        assert!(payload.get("text").is_none());
        assert!(payload.get("blocks").is_none());
        assert_eq!(payload["attachments"].as_array().unwrap().len(), 1);
        let attachment = &payload["attachments"][0];
        assert!(attachment["color"].as_str().is_some_and(|c| c.starts_with('#')));
        assert!(attachment["fallback"].as_str().is_some_and(|text| !text.is_empty()));
        assert!(attachment["blocks"].as_array().is_some_and(|b| !b.is_empty()));
        let rendered = rendered_blocks(&payload);
        assert_eq!(rendered.matches("the export port is not answering").count(), 1);
        assert_eq!(rendered.matches("確認手順").count(), 1);
        assert_eq!(rendered.matches("systemctl status nfs-server").count(), 1);
        assert!(!attachment["fallback"].as_str().unwrap().contains("確認手順"));
    }

    #[test]
    fn completed_and_incomplete_recovery_have_distinct_colors() {
        assert_eq!(WebhookProvider::color(&notification(Trigger::Resolved)), "#2e7d32");
        assert_eq!(WebhookProvider::color(&notification(Trigger::Recovering)), "#f9a825");
        assert_eq!(WebhookProvider::color(&notification(Trigger::Opened)), "#d32f2f");
    }

    #[test]
    fn only_red_critical_notifications_begin_with_a_channel_mention() {
        for severity in [Severity::Info, Severity::Warning, Severity::Critical] {
            for trigger in [
                Trigger::Opened,
                Trigger::Escalated,
                Trigger::DiagnosisChanged,
                Trigger::Recovering,
                Trigger::Resolved,
            ] {
                let mut notification = notification(trigger);
                notification.severity = severity;
                let payload = WebhookProvider::slack_payload(&notification);
                let red = payload["attachments"][0]["color"] == "#d32f2f";
                assert!(payload.get("text").is_none());
                assert_eq!(
                    payload["attachments"][0]["fallback"]
                        .as_str()
                        .unwrap()
                        .starts_with("<!channel>\n"),
                    red
                );
                let first = &payload["attachments"][0]["blocks"][0];
                assert_eq!(first["text"]["text"] == "<!channel>", red);
                if red {
                    assert_eq!(first["text"]["type"], "mrkdwn");
                }
                assert_eq!(
                    rendered_blocks(&payload).matches("<!channel>").count(),
                    usize::from(red)
                );
            }
        }
    }

    #[test]
    fn recovery_is_not_reported_as_complete_until_resolved() {
        let recovering = notification(Trigger::Recovering);
        assert_eq!(WebhookProvider::payload(&recovering)["resolved"], false);
        let payload = WebhookProvider::slack_payload(&recovering);
        assert_eq!(payload["attachments"][0]["blocks"][0]["text"]["text"], "復旧途中");
        assert!(rendered_blocks(&payload).contains("まだ確認できていません"));
    }

    #[test]
    fn long_japanese_instructions_keep_every_command_in_order() {
        let mut notification = notification(Trigger::Opened);
        notification.recommended_actions = (0..40)
            .map(|i| {
                format!(
                    "{}\nsudo -u sentinel sentinel entity show 'host/node{i}' --json",
                    "接続先と観測時刻を確認してください。".repeat(12)
                )
            })
            .collect();
        let payload = WebhookProvider::slack_payload(&notification);
        let rendered = rendered_blocks(&payload);
        let mut offset = 0;
        for command in commands(&payload) {
            let next = rendered[offset..].find(command).expect("complete command");
            offset += next + command.len();
        }
        assert!(!rendered.contains("一部を省略"));
        assert_eq!(commands(&payload).len(), 40);
    }

    #[test]
    fn commands_use_code_blocks_and_guidance_remains_literal_text() {
        let mut notification = notification(Trigger::Opened);
        let command =
            "sudo -u sentinel sentinel entity show 'host/名前```<!channel>&' --json\n\nprintf '%s\\n' '<@U123>'";
        let guidance = "SSHで入れない場合は、コンソールやBMCからログインしてください。";
        notification.recommended_actions = vec![
            format!("対象ホストで、接続先を確認してください。\n{command}"),
            guidance.into(),
            "systemctl status sshd  # on node01".into(),
            "journalctl -u sshd -n 100  # on node01".into(),
            "ss -lntp\nip address show".into(),
        ];
        let payload = WebhookProvider::slack_payload(&notification);
        assert_eq!(
            commands(&payload),
            [
                command,
                "systemctl status sshd  # on node01",
                "journalctl -u sshd -n 100  # on node01",
                "ss -lntp\nip address show",
            ]
        );
        let elements: Vec<_> = payload["attachments"][0]["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|block| block["type"] == "rich_text")
            .flat_map(|block| block["elements"].as_array().into_iter().flatten())
            .collect();
        let guidance = elements
            .iter()
            .find(|element| element["elements"][0]["text"] == format!("2. {guidance}"))
            .expect("guidance stays outside code blocks");
        assert_eq!(guidance["type"], "rich_text_section");
        // Every rich text leaf is literal text, never a mention or a link.
        assert!(elements.iter().all(|element| element["elements"]
            .as_array()
            .unwrap()
            .iter()
            .all(|leaf| leaf["type"] == "text")));
        assert_eq!(
            WebhookProvider::payload(&notification)["recommended_actions"],
            serde_json::json!(notification.recommended_actions)
        );
    }

    #[test]
    fn instructions_separate_location_sentences_commands_and_next_step() {
        let mut notification = notification(Trigger::Opened);
        notification.recommended_actions = vec![
            crate::diagnosis::investigation::step(
                "node01",
                "SSHサービスの状態を確認してください。サービス名がsshの環境ではsshdをsshに置き換えてください",
                "sudo systemctl status sshd --no-pager -l",
            ),
            crate::diagnosis::investigation::step(
                "node01",
                "SSHのログを確認してください",
                "sudo journalctl -u sshd -n 100 --no-pager",
            ),
        ];
        let payload = WebhookProvider::slack_payload(&notification);
        let rendered = rendered_blocks(&payload);
        assert!(rendered.contains(
            "1. 実行先　node01\nSSHサービスの状態を確認してください。\nサービス名がsshの環境ではsshdをsshに置き換えてください。\nsudo systemctl status sshd --no-pager -l\n\n\n2. 実行先　node01\nSSHのログを確認してください。"
        ));
        assert_eq!(
            commands(&payload),
            [
                "sudo systemctl status sshd --no-pager -l",
                "sudo journalctl -u sshd -n 100 --no-pager",
            ]
        );
        assert!(rendered.contains("診断の確度　高\n\nthe export port is not answering\n\n障害ID"));
    }

    #[test]
    fn overflow_preserves_the_cli_fallback_and_never_splits_a_command() {
        let mut notification = notification(Trigger::Opened);
        let command = format!("sudo -u sentinel sentinel entity show {}", "巨大な名前".repeat(1000));
        notification.recommended_actions = (0..100)
            .map(|_| "確認してください。".repeat(300))
            .chain([command.clone()])
            .collect();
        let payload = WebhookProvider::slack_payload(&notification);
        let blocks = payload["attachments"][0]["blocks"].as_array().unwrap();
        assert!(blocks.len() <= 50);
        let texts: Vec<_> = blocks.iter().map(block_text).collect();
        assert!(texts
            .iter()
            .any(|t| t.contains("sudo -u sentinel sentinel incident show")));
        assert!(texts.iter().all(|t| !t.contains("巨大な名前")));
        assert!(texts.iter().all(|t| t.chars().count() <= SLACK_SECTION_LIMIT));
        assert!(commands(&payload)
            .iter()
            .any(|command| command.starts_with("sudo -u sentinel sentinel incident show")));
    }

    #[test]
    fn a_test_notification_keeps_its_test_heading() {
        let mut notification = notification(Trigger::Opened);
        notification.title = "通知テスト / Cluster Sentinel".into();
        let payload = WebhookProvider::slack_payload(&notification);
        assert_eq!(payload["attachments"][0]["blocks"][1]["text"]["text"], "通知テスト");
    }

    #[test]
    fn the_heading_does_not_repeat_what_the_colour_says() {
        // Titles are `SEVERITY: summary`, and the heading already carries the
        // severity beside a coloured bar. Repeating it wastes the first line,
        // which on a phone is most of what gets read.
        let mut opened = notification(Trigger::Opened);
        opened.title = "CRITICAL: filesrv01 is up but the export port is not answering".into();
        assert_eq!(
            WebhookProvider::summary(&opened),
            "filesrv01 is up but the export port is not answering"
        );

        // A title with no prefix is left alone rather than losing its first clause.
        opened.title = "something happened".into();
        assert_eq!(WebhookProvider::summary(&opened), "something happened");
    }

    #[test]
    fn external_text_is_literal_and_cannot_add_a_mention() {
        let mut n = notification(Trigger::Opened);
        n.body = "before ``` <!channel> after".into();
        n.title = format!("診断更新（警告） / {}", n.body);
        n.severity = Severity::Warning;
        let payload = WebhookProvider::slack_payload(&n);
        let blocks = payload["attachments"][0]["blocks"].as_array().unwrap();
        let body = blocks
            .iter()
            .find(|b| b["text"]["text"] == n.body)
            .expect("literal body");
        assert_eq!(body["text"]["type"], "plain_text");
        let fallback = payload["attachments"][0]["fallback"].as_str().unwrap();
        assert!(!fallback.contains("<!channel>"));
        assert!(fallback.contains("&lt;!channel&gt;"));
    }

    #[test]
    fn blocks_stay_inside_slacks_limits() {
        // Slack rejects an over-long header rather than truncating it, so a
        // long diagnosis would mean no notification at all.
        let mut long = notification(Trigger::Opened);
        long.title = "X".repeat(400);
        long.body = "Y".repeat(8000);
        long.recommended_actions = vec!["Z".repeat(5000)];

        let payload = WebhookProvider::slack_payload(&long);
        for block in payload["attachments"][0]["blocks"].as_array().expect("blocks") {
            assert!(block_text(block).chars().count() <= SLACK_SECTION_LIMIT);
            if let Some(text) = block["text"]["text"].as_str() {
                let limit = if block["type"] == "header" {
                    SLACK_HEADER_LIMIT
                } else {
                    SLACK_SECTION_LIMIT
                };
                assert!(
                    text.chars().count() <= limit,
                    "{} block: {} chars",
                    block["type"],
                    text.chars().count()
                );
            }
        }
    }

    #[test]
    fn the_format_decides_the_shape() {
        let n = notification(Trigger::Opened);
        assert!(WebhookProvider::payload_for(WebhookFormat::Slack, &n)["attachments"].is_array());
        assert!(WebhookProvider::payload_for(WebhookFormat::Generic, &n)["attachments"].is_null());
        // Generic keeps the structured fields a script would route on.
        assert_eq!(
            WebhookProvider::payload_for(WebhookFormat::Generic, &n)["source"],
            "cluster-sentinel"
        );
    }

    #[test]
    fn the_payload_carries_a_message_slack_will_accept() {
        // Slack refuses a payload with no `text`:
        // `missing_text_or_fallback_or_attachments`. Reported from a real
        // cluster, where the only destination configured was a Slack webhook
        // and every notification bounced with a 400.
        let payload = WebhookProvider::payload(&notification(Trigger::Opened));

        let text = payload["text"].as_str().expect("text");
        assert!(!text.is_empty());
        assert!(text.contains(&notification(Trigger::Opened).title));

        // Discord wants the same thing under another name.
        assert_eq!(payload["content"], payload["text"]);
    }

    #[test]
    fn the_message_includes_what_to_do_about_it() {
        let notification = notification(Trigger::Opened);
        let payload = WebhookProvider::payload(&notification);
        let text = payload["text"].as_str().expect("text");
        for action in &notification.recommended_actions {
            assert!(text.contains(action), "{text}");
        }
    }

    #[test]
    fn the_structured_fields_are_still_there() {
        // The message is carried alongside them, not instead of them: a
        // receiver that routes on severity must keep working.
        let payload = WebhookProvider::payload(&notification(Trigger::Resolved));
        assert_eq!(payload["source"], "cluster-sentinel");
        assert_eq!(payload["resolved"], true);
        assert!(payload["severity"].is_string());
        assert!(payload["fingerprint"].is_string());
    }

    #[test]
    fn a_url_without_a_scheme_is_refused_at_construction() {
        // Better to fail at startup than to discover it during an outage.
        let error =
            WebhookProvider::new("webhook", "example.org/hook", Duration::from_secs(5)).expect_err("must refuse");
        assert!(matches!(error, ProviderError::Misconfigured(_)));
        assert!(!error.is_transient());
    }

    #[test]
    fn both_schemes_are_accepted() {
        for url in ["http://example.org/hook", "https://example.org/hook"] {
            assert!(
                WebhookProvider::new("webhook", url, Duration::from_secs(5)).is_ok(),
                "{url}"
            );
        }
    }

    #[test]
    fn the_payload_is_self_describing() {
        // A receiver should be able to route on severity and recognise a
        // recovery without knowing anything about Sentinel.
        let payload = WebhookProvider::payload(&notification(Trigger::Opened));

        assert_eq!(payload["source"], "cluster-sentinel");
        assert_eq!(payload["severity"], "critical");
        assert_eq!(payload["trigger"], "opened");
        assert_eq!(payload["resolved"], false);
        assert!(payload["title"].as_str().unwrap().contains("重大"));
        assert!(payload["recommended_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "systemctl status nfs-server"));
    }

    #[test]
    fn a_recovery_is_flagged_as_such_in_the_payload() {
        let payload = WebhookProvider::payload(&notification(Trigger::Resolved));
        assert_eq!(payload["resolved"], true);
        assert_eq!(payload["trigger"], "resolved");
    }

    #[tokio::test]
    async fn a_notification_reaches_a_listening_endpoint() {
        let received = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<serde_json::Value>::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();

        {
            let received = std::sync::Arc::clone(&received);
            tokio::spawn(async move {
                let app = axum::Router::new().route(
                    "/hook",
                    axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
                        let received = std::sync::Arc::clone(&received);
                        async move {
                            received.lock().await.push(body);
                            "ok"
                        }
                    }),
                );
                let _ = axum::serve(listener, app).await;
            });
        }

        let provider = WebhookProvider::new(
            "webhook",
            format!("http://127.0.0.1:{port}/hook"),
            Duration::from_secs(2),
        )
        .expect("provider");
        provider
            .send(&notification(Trigger::Opened))
            .await
            .expect("generic send");
        provider
            .with_format(WebhookFormat::Slack)
            .send(&notification(Trigger::Opened))
            .await
            .expect("Slack send");

        let received = received.lock().await;
        assert_eq!(received.len(), 2);
        assert_eq!(received[0]["fingerprint"], "cause:fs1");
        assert!(received[1].get("text").is_none());
        assert_eq!(received[1]["attachments"][0]["blocks"][0]["text"]["text"], "<!channel>");
        assert_eq!(received[1]["attachments"][0]["color"], "#d32f2f");
        assert!(commands(&received[1]).contains(&"systemctl status nfs-server"));
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_is_a_transient_error() {
        // The agent should keep trying; a webhook being down is not a reason
        // to stop monitoring.
        let provider =
            WebhookProvider::new("webhook", "http://127.0.0.1:1/hook", Duration::from_millis(200)).expect("provider");
        let error = provider
            .send(&notification(Trigger::Opened))
            .await
            .expect_err("must fail");

        assert!(matches!(error, ProviderError::Unreachable { .. }), "{error:?}");
        assert!(error.is_transient());
    }

    #[test]
    fn permanent_and_transient_failures_are_distinguished() {
        assert!(ProviderError::Rejected {
            destination: String::new(),
            status: 503,
            detail: String::new()
        }
        .is_transient());
        assert!(ProviderError::Rejected {
            destination: String::new(),
            status: 429,
            detail: String::new()
        }
        .is_transient());
        assert!(!ProviderError::Rejected {
            destination: String::new(),
            status: 400,
            detail: String::new()
        }
        .is_transient());
    }
}

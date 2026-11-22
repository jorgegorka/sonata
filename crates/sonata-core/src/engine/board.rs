use super::*;

impl Engine {
    pub fn mark_session_seen(&self, session_id: &str) -> Result<()> {
        let session =
            self.store.set_session_seen(session_id)?.ok_or_else(|| Refusal::SessionNotFound(session_id.into()))?;
        self.emit(UiEvent::SessionUpdated { session: self.session_view(session) });
        Ok(())
    }

    pub async fn session_activity(&self, session_ids: &[String]) -> Result<Vec<SessionActivity>> {
        let in_flight: HashMap<String, String> = {
            let live = self.inner.live.lock().await;
            session_ids
                .iter()
                .filter_map(|id| {
                    Some((id.clone(), {
                        let l = live.get(id)?;
                        l.in_flight.as_ref().or(l.limited_turn.as_ref())?.message_id.clone()
                    }))
                })
                .collect()
        };
        let mut out = Vec::new();
        for id in session_ids {
            let Some(rows) = self.store.activity_rows(id, in_flight.get(id).map(String::as_str))? else {
                continue;
            };
            let cut_off = rows.last_to.filter(|m| m.status == MessageStatus::Interrupted).map(|m| CutOff {
                at: m.delivered_at.unwrap_or_else(|| m.created_at.clone()),
                message_id: m.id,
                sender: m.sender,
            });
            out.push(SessionActivity {
                session_id: id.clone(),
                working_on: rows.in_flight.map(|m| CardLine {
                    text: first_line(&m.body, LINE_MAX).unwrap_or_default(),
                    at: m.delivered_at.unwrap_or(m.created_at),
                }),
                queued: rows.queued,
                question: rows.question.map(|m| m.body),
                last_sent: rows.last_sent.map(|m| SentLine {
                    recipient: m.recipient,
                    text: first_line(&m.body, LINE_MAX).unwrap_or_default(),
                    at: m.created_at,
                }),
                cut_off,
                last_reply: rows.last_reply.map(|m| Reply {
                    text: preview(&m.body, PREVIEW_MAX),
                    message_id: m.id,
                    files: m.attachments.into_iter().map(|a| a.name).collect(),
                    at: m.created_at,
                    raw: m.kind == MessageKind::RawFallback,
                }),
                last_for_user_at: rows.last_for_user_at,
                run_of: rows.run_of,
                workflow: self.run_glance_of(id)?,
            });
        }
        Ok(out)
    }

    pub async fn session_glances(&self, session_ids: &[String]) -> Vec<Glance> {
        let mut out = Vec::new();
        for id in session_ids {
            let Some((status, rt)) = ({
                let live = self.inner.live.lock().await;
                live.get(id).and_then(|l| Some((l.status, l.rt.clone()?)))
            }) else {
                continue;
            };
            if !matches!(status, BotStatus::Working | BotStatus::Idle | BotStatus::NeedsAttention) {
                continue;
            }
            let Some(adapter) = self.session_with_bot(id).and_then(|(bot, _)| adapter_for(bot.provider)) else {
                continue;
            };
            let tells = match status {
                BotStatus::Working => adapter.reads_step(),
                BotStatus::Idle => adapter.reads_background_work() && adapter.wakes_after_background_work(),
                _ => true,
            };
            if !tells {
                continue;
            }
            let screen = match self.inner.runtime.snapshot(&rt, super::controls::SNAPSHOT_LINES).await {
                Ok(screen) => screen,
                Err(e) => {
                    let who = self.who(id);
                    tracing::warn!(bot = %who.bot, session = who.session, error = %e, "could not read the session's screen for its card");
                    continue;
                }
            };
            out.push(Glance {
                session_id: id.clone(),
                step: (status == BotStatus::Working)
                    .then(|| adapter.current_step(&screen))
                    .flatten()
                    .and_then(|step| first_line(&step, LINE_MAX)),
                background_work: status == BotStatus::Idle
                    && adapter.wakes_after_background_work()
                    && adapter.background_work(&screen),
                tail: (status == BotStatus::NeedsAttention).then(|| last_lines(&screen, TAIL_LINES)).flatten(),
            });
        }
        out
    }
}

const TAIL_LINES: usize = 6;

fn last_lines(screen: &str, count: usize) -> Option<String> {
    let lines: Vec<&str> = screen.lines().map(str::trim_end).filter(|line| !line.trim().is_empty()).collect();
    (!lines.is_empty()).then(|| lines[lines.len().saturating_sub(count)..].join("\n"))
}

const LINE_MAX: usize = 120;
const PREVIEW_MAX: usize = 280;

fn preview(body: &str, max: usize) -> String {
    let text = body.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max - 1).chain(['…']).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_lines_skip_blank_lines_and_cut_long_ones() {
        assert_eq!(first_line("\n  \n  Fix it  \nmore", LINE_MAX).as_deref(), Some("Fix it"));
        let long = first_line(&"x".repeat(2000), LINE_MAX).unwrap();
        assert_eq!(long.chars().count(), LINE_MAX);
        assert!(long.ends_with('…'));
        assert_eq!(first_line("  \n ", LINE_MAX), None);
        assert_eq!(preview(&"y".repeat(400), PREVIEW_MAX).chars().count(), PREVIEW_MAX);
    }
}

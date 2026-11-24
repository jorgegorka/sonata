use bridge_proto::*;
use serde_json::json;

#[test]
fn onboarding_proposal_keeps_generation_and_json_body_separate() {
    let body = json!({"workflows": [{"name": "Research\nreport"}]}).to_string();
    let request = BridgeRequest {
        v: PROTOCOL_VERSION,
        pane_id: "w1:p2".into(),
        cli_session_id: None,
        command: BridgeCommand::OnboardingPropose { generation: "g1".into(), body: body.clone() },
    };
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["cmd"], "onboarding_propose");
    assert_eq!(value["body"], body);
    assert_eq!(serde_json::from_value::<BridgeRequest>(value).unwrap(), request);
}

#[test]
fn reply_request_matches_spec_json() {
    let req = BridgeRequest {
        v: PROTOCOL_VERSION,
        pane_id: "w1:p3".into(),
        cli_session_id: None,
        command: BridgeCommand::Reply { body: "hi".into(), files: vec![] },
    };
    let value = serde_json::to_value(&req).unwrap();
    assert_eq!(value, json!({"v":1,"pane_id":"w1:p3","cmd":"reply","body":"hi","files":[]}));
    let back: BridgeRequest = serde_json::from_value(value).unwrap();
    assert_eq!(back, req);
}

#[test]
fn send_request_parses_without_files() {
    let req: BridgeRequest =
        serde_json::from_str(r#"{"v":1,"pane_id":"w1:p3","cmd":"send","to":"@writer","body":"x"}"#).unwrap();
    assert_eq!(req.command, BridgeCommand::Send { to: "@writer".into(), body: "x".into(), files: vec![] });
}

#[test]
fn bots_request_parses() {
    let req: BridgeRequest = serde_json::from_str(r#"{"v":1,"pane_id":"w1:p3","cmd":"bots"}"#).unwrap();
    assert_eq!(req.command, BridgeCommand::Bots);
}

#[test]
fn sent_response_names_the_recipient_and_omits_empty_fields() {
    let s = serde_json::to_string(&BridgeResponse::sent("01J", "@writer")).unwrap();
    assert_eq!(s, r#"{"ok":true,"message_id":"01J","to":"@writer"}"#);
}

#[test]
fn a_response_without_to_still_parses() {
    let r: BridgeResponse = serde_json::from_str(r#"{"ok":true,"message_id":"01J"}"#).unwrap();
    assert_eq!((r.message_id.as_deref(), r.to), (Some("01J"), None));
}

#[test]
fn bots_response_round_trips() {
    let r = BridgeResponse::bots(vec![BotSummary {
        handle: "writer".into(),
        display_name: "Writer".into(),
        provider: "grok".into(),
        status: "idle".into(),
        role: "Writes drafts.".into(),
        waiting_for_user: false,
    }]);
    let back: BridgeResponse = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
    assert_eq!(back, r);
}

#[test]
fn error_response_matches_spec_json() {
    let r = BridgeResponse::error(ErrorCode::UnknownRecipient, "No bot named @writr.");
    assert_eq!(
        serde_json::to_value(&r).unwrap(),
        json!({"ok":false,"error":{"code":"unknown_recipient","message":"No bot named @writr."}})
    );
}

#[test]
fn a_request_carries_the_cli_session_id_only_when_it_has_one() {
    let req = BridgeRequest {
        v: 1,
        pane_id: "w1:p3".into(),
        cli_session_id: Some("01a1".into()),
        command: BridgeCommand::Bots,
    };
    assert_eq!(
        serde_json::to_value(&req).unwrap(),
        json!({"v":1,"pane_id":"w1:p3","cli_session_id":"01a1","cmd":"bots"})
    );
    let old: BridgeRequest = serde_json::from_str(r#"{"v":1,"pane_id":"w1:p3","cmd":"bots"}"#).unwrap();
    assert_eq!(old.cli_session_id, None);
}

#[test]
fn a_field_this_side_does_not_know_is_ignored() {
    let req: BridgeRequest =
        serde_json::from_str(r#"{"v":1,"pane_id":"w1:p3","cmd":"reply","body":"x","from_the_future":true}"#).unwrap();
    assert_eq!(req.command, BridgeCommand::Reply { body: "x".into(), files: vec![] });
}

#[test]
fn new_commands_round_trip_and_default_their_fields() {
    let ask: BridgeRequest = serde_json::from_str(r#"{"v":1,"pane_id":"p","cmd":"ask","body":"Which date?"}"#).unwrap();
    assert_eq!(ask.command, BridgeCommand::Ask { body: "Which date?".into(), files: vec![], options: vec![] });
    let create: BridgeRequest = serde_json::from_str(
        r##"{"v":1,"pane_id":"p","cmd":"bot_create","name":"SEO","color":"#111111","instructions":"SEO.\n"}"##,
    )
    .unwrap();
    assert_eq!(
        create.command,
        BridgeCommand::BotCreate {
            name: "SEO".into(),
            color: "#111111".into(),
            provider: "".into(),
            skills: vec![],
            instructions: "SEO.\n".into(),
            recreate: false,
            model: None,
        }
    );
    for (cmd, json) in [
        (BridgeCommand::Result { body: "Done.".into(), files: vec![], outcome: None }, r#""cmd":"result""#),
        (BridgeCommand::BotShow { handle: "@plan".into() }, r#""cmd":"bot_show""#),
        (BridgeCommand::Skills, r#""cmd":"skills""#),
        (BridgeCommand::SkillCopy { slug: "sonata-plan".into() }, r#""cmd":"skill_copy""#),
        (BridgeCommand::SkillAdd { folder: "/x".into(), recreate: false }, r#""cmd":"skill_add""#),
    ] {
        let req = BridgeRequest { v: 1, pane_id: "p".into(), cli_session_id: None, command: cmd };
        let text = serde_json::to_string(&req).unwrap();
        assert!(text.contains(json), "{text}");
        assert_eq!(serde_json::from_str::<BridgeRequest>(&text).unwrap(), req);
    }
}

#[test]
fn old_responses_still_parse_and_new_fields_are_omitted_when_empty() {
    let r: BridgeResponse = serde_json::from_str(r#"{"ok":true,"message_id":"m","to":"user"}"#).unwrap();
    assert_eq!((r.bot, r.skills, r.draft, r.created), (None, None, None, None));
    let text = serde_json::to_string(&BridgeResponse::created("seo", false)).unwrap();
    assert_eq!(text, r#"{"ok":true,"created":{"name":"seo","already":false}}"#);
    let summary = BotSummary {
        handle: "plan".into(),
        display_name: "Plan".into(),
        provider: "claude".into(),
        status: "idle".into(),
        role: "Plans.".into(),
        waiting_for_user: false,
    };
    assert!(!serde_json::to_string(&summary).unwrap().contains("waiting_for_user"));
}

#[test]
fn a_skills_response_names_the_drafts_folder_and_others_omit_it() {
    let skill = SkillSummary { slug: "seo".into(), name: "SEO".into(), description: "SEO.".into(), used_by: vec![] };
    let r = BridgeResponse::skills(vec![skill], "/bots/master/skill-drafts/session-1");
    assert_eq!(r.drafts_dir.as_deref(), Some("/bots/master/skill-drafts/session-1"));
    let text = serde_json::to_string(&r).unwrap();
    assert!(text.contains(r#""drafts_dir":"/bots/master/skill-drafts/session-1""#), "{text}");
    assert_eq!(serde_json::from_str::<BridgeResponse>(&text).unwrap(), r);
    let draft = serde_json::to_string(&BridgeResponse::draft("/d/seo", true)).unwrap();
    assert_eq!(draft, r#"{"ok":true,"draft":{"path":"/d/seo","resumed":true}}"#);
    let old: BridgeResponse = serde_json::from_str(r#"{"ok":true,"skills":[]}"#).unwrap();
    assert_eq!(old.drafts_dir, None);
}

#[test]
fn task_session_skill_update_and_bot_edit_commands_round_trip() {
    for (cmd, json) in [
        (BridgeCommand::Tasks, r#""cmd":"tasks""#),
        (
            BridgeCommand::TaskCreate {
                name: "Daily".into(),
                bot: "@plan".into(),
                every: 1,
                at: Some("09:00".into()),
                from: None,
                prompt: "Go.\n".into(),
                recreate: false,
            },
            r#""cmd":"task_create""#,
        ),
        (
            BridgeCommand::TaskEdit {
                task: "daily".into(),
                name: None,
                bot: None,
                every: Some(2),
                at: None,
                from: Some("2026-11-01".into()),
                enabled: Some(false),
                prompt: None,
            },
            r#""cmd":"task_edit""#,
        ),
        (BridgeCommand::TaskDelete { task: "daily".into() }, r#""cmd":"task_delete""#),
        (BridgeCommand::Sessions { archived: true, all: false }, r#""cmd":"sessions""#),
        (BridgeCommand::SessionShow { session: "@plan/1".into() }, r#""cmd":"session_show""#),
        (BridgeCommand::SessionArchive { sessions: vec!["@plan/1".into()] }, r#""cmd":"session_archive""#),
        (BridgeCommand::SkillUpdate { slug: "seo".into(), folder: "/x".into() }, r#""cmd":"skill_update""#),
        (
            BridgeCommand::BotEdit {
                handle: "@plan".into(),
                instructions: Some("New.".into()),
                add_skills: vec!["seo".into()],
                remove_skills: vec![],
                model: None,
            },
            r#""cmd":"bot_edit""#,
        ),
    ] {
        let req = BridgeRequest { v: 1, pane_id: "p".into(), cli_session_id: None, command: cmd };
        let text = serde_json::to_string(&req).unwrap();
        assert!(text.contains(json), "{text}");
        assert_eq!(serde_json::from_str::<BridgeRequest>(&text).unwrap(), req);
    }
}

#[test]
fn optional_fields_of_the_new_commands_default() {
    let c: BridgeCommand = serde_json::from_str(r#"{"cmd":"task_edit","task":"x"}"#).unwrap();
    assert_eq!(
        c,
        BridgeCommand::TaskEdit {
            task: "x".into(),
            name: None,
            bot: None,
            every: None,
            at: None,
            from: None,
            enabled: None,
            prompt: None
        }
    );
    let c: BridgeCommand = serde_json::from_str(r#"{"cmd":"sessions"}"#).unwrap();
    assert_eq!(c, BridgeCommand::Sessions { archived: false, all: false });
    let c: BridgeCommand = serde_json::from_str(r#"{"cmd":"bot_edit","handle":"@a"}"#).unwrap();
    assert_eq!(
        c,
        BridgeCommand::BotEdit {
            handle: "@a".into(),
            instructions: None,
            add_skills: vec![],
            remove_skills: vec![],
            model: None
        }
    );
}

#[test]
fn an_output_response_round_trips_and_old_ones_have_none() {
    let r = BridgeResponse::output("one\ntwo");
    let text = serde_json::to_string(&r).unwrap();
    assert_eq!(text, r#"{"ok":true,"output":"one\ntwo"}"#);
    assert_eq!(serde_json::from_str::<BridgeResponse>(&text).unwrap(), r);
    let old: BridgeResponse = serde_json::from_str(r#"{"ok":true}"#).unwrap();
    assert_eq!(old.output, None);
}

#[test]
fn ask_carries_its_options_in_order() {
    let line = r#"{"v":1,"pane_id":"p","cmd":"ask","body":"Plan?","options":["Approve","Change the plan"]}"#;
    let req: BridgeRequest = serde_json::from_str(line).unwrap();
    assert_eq!(
        req.command,
        BridgeCommand::Ask {
            body: "Plan?".into(),
            files: vec![],
            options: vec!["Approve".into(), "Change the plan".into()]
        }
    );
    let again: BridgeRequest = serde_json::from_str(&serde_json::to_string(&req).unwrap()).unwrap();
    assert_eq!(again, req);
}

#[test]
fn a_result_carries_its_outcome_and_one_without_leaves_it_out() {
    let old: BridgeRequest = serde_json::from_str(r#"{"v":1,"pane_id":"p","cmd":"result","body":"Done."}"#).unwrap();
    assert_eq!(old.command, BridgeCommand::Result { body: "Done.".into(), files: vec![], outcome: None });
    assert!(!serde_json::to_string(&old).unwrap().contains("outcome"));
    let req = BridgeRequest {
        v: 1,
        pane_id: "p".into(),
        cli_session_id: None,
        command: BridgeCommand::Result { body: "LGTM".into(), files: vec![], outcome: Some("approved".into()) },
    };
    let text = serde_json::to_string(&req).unwrap();
    assert!(text.contains(r#""outcome":"approved""#), "{text}");
    assert_eq!(serde_json::from_str::<BridgeRequest>(&text).unwrap(), req);
}

#[test]
fn workflow_commands_round_trip_and_default_their_fields() {
    for (cmd, json) in [
        (BridgeCommand::Workflows, r#""cmd":"workflows""#),
        (BridgeCommand::WorkflowShow { name: "Flow".into() }, r#""cmd":"workflow_show""#),
        (
            BridgeCommand::WorkflowRun { name: "Flow".into(), body: "TASK-1".into(), files: vec!["/a.md".into()] },
            r#""cmd":"workflow_run""#,
        ),
        (
            BridgeCommand::WorkflowCreate { definition: "Workflow: Flow\n".into(), recreate: true },
            r#""cmd":"workflow_create""#,
        ),
        (
            BridgeCommand::WorkflowEdit { name: "Flow".into(), definition: "Workflow: Flow\n".into() },
            r#""cmd":"workflow_edit""#,
        ),
        (BridgeCommand::WorkflowDelete { name: "Flow".into() }, r#""cmd":"workflow_delete""#),
    ] {
        let req = BridgeRequest { v: 1, pane_id: "p".into(), cli_session_id: None, command: cmd };
        let text = serde_json::to_string(&req).unwrap();
        assert!(text.contains(json), "{text}");
        assert_eq!(serde_json::from_str::<BridgeRequest>(&text).unwrap(), req);
    }
    let c: BridgeCommand = serde_json::from_str(r#"{"cmd":"workflow_run","name":"Flow","body":"Go."}"#).unwrap();
    assert_eq!(c, BridgeCommand::WorkflowRun { name: "Flow".into(), body: "Go.".into(), files: vec![] });
    let c: BridgeCommand = serde_json::from_str(r#"{"cmd":"workflow_create","definition":"x"}"#).unwrap();
    assert_eq!(c, BridgeCommand::WorkflowCreate { definition: "x".into(), recreate: false });
}

#[test]
fn bot_create_and_edit_carry_a_model_only_when_given() {
    for (cmd, json) in [
        (
            BridgeCommand::BotCreate {
                name: "Scout".into(),
                color: "#111111".into(),
                provider: "opencode".into(),
                skills: vec![],
                instructions: "Scouts.".into(),
                recreate: false,
                model: Some("opencode-go/kimi-k3".into()),
            },
            r#""model":"opencode-go/kimi-k3""#,
        ),
        (
            BridgeCommand::BotEdit {
                handle: "@scout".into(),
                instructions: None,
                add_skills: vec![],
                remove_skills: vec![],
                model: Some(String::new()),
            },
            r#""model":"""#,
        ),
    ] {
        let req = BridgeRequest { v: 1, pane_id: "p".into(), cli_session_id: None, command: cmd };
        let text = serde_json::to_string(&req).unwrap();
        assert!(text.contains(json), "{text}");
        assert_eq!(serde_json::from_str::<BridgeRequest>(&text).unwrap(), req);
    }
    let edit = BridgeCommand::BotEdit {
        handle: "@scout".into(),
        instructions: None,
        add_skills: vec![],
        remove_skills: vec![],
        model: None,
    };
    assert!(!serde_json::to_string(&edit).unwrap().contains("model"), "an older app gets the request it knows");
    let old: BridgeCommand =
        serde_json::from_str(r##"{"cmd":"bot_create","name":"S","color":"#111111","instructions":"S."}"##).unwrap();
    assert!(matches!(old, BridgeCommand::BotCreate { model: None, .. }), "{old:?}");
}

#[test]
fn a_bot_detail_names_its_model_only_when_it_has_one() {
    let detail = BotDetail {
        handle: "scout".into(),
        display_name: "Scout".into(),
        provider: "opencode".into(),
        status: "idle".into(),
        role: "Scouts.".into(),
        instructions: "Scouts.".into(),
        skills: vec![],
        model: None,
    };
    assert!(!serde_json::to_string(&detail).unwrap().contains("model"));
    let with = BotDetail { model: Some("opencode-go/kimi-k3".into()), ..detail };
    let text = serde_json::to_string(&with).unwrap();
    assert!(text.contains(r#""model":"opencode-go/kimi-k3""#), "{text}");
    assert_eq!(serde_json::from_str::<BotDetail>(&text).unwrap(), with);
}

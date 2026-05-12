use anyhow::{bail, Result};
use dialoguer::Input;
use serde_json::{json, Value as JsonValue};

use crate::cli::{AiAction, InvestigateTarget};
use crate::context::AppContext;

pub async fn run_ai_command(ctx: &AppContext, action: AiAction) -> Result<()> {
    match action {
        AiAction::Status => {
            let spinner = ctx.ui.spinner("Checking AI provider status");
            let payload = ctx
                .api_request(reqwest::Method::GET, "/api/ai/status", None)
                .await?;
            spinner.finish("AI status ready");
            render_ai_status(ctx, &payload);
            Ok(())
        }
        AiAction::Doctor => {
            let spinner = ctx.ui.spinner("Running AI doctor");
            let payload = ctx
                .api_request(reqwest::Method::GET, "/api/ai/doctor", None)
                .await?;
            spinner.finish("AI doctor completed");
            render_ai_doctor(ctx, &payload);
            Ok(())
        }
        AiAction::Ask {
            question,
            scope,
            mode,
            monitor_seconds,
        } => {
            let question = resolve_question(ctx, question)?;
            let spinner = ctx.ui.spinner("Asking Inferra AI");
            let mut body = json!({ "question": question, "scope": scope, "mode": mode });
            if let Some(ms) = monitor_seconds {
                body["monitor_seconds"] = json!(ms);
            }
            let payload = ctx
                .api_request(reqwest::Method::POST, "/api/ai/ask", Some(body))
                .await?;
            spinner.finish("Investigation response ready");
            render_investigation(ctx, &payload);
            Ok(())
        }
        AiAction::Report {
            incident_id,
            mode,
            monitor_seconds,
        } => {
            let spinner = ctx.ui.spinner("Generating incident report");
            let mut query = mode
                .as_ref()
                .map(|value| format!("?mode={value}"))
                .unwrap_or_default();
            if let Some(ms) = monitor_seconds {
                if query.is_empty() {
                    query.push('?');
                } else {
                    query.push('&');
                }
                query.push_str(&format!("monitor_seconds={ms}"));
            }
            let payload = ctx
                .api_request(
                    reqwest::Method::GET,
                    &format!("/api/ai/report/{incident_id}{query}"),
                    None,
                )
                .await?;
            spinner.finish("Incident report ready");
            render_investigation(ctx, &payload);
            Ok(())
        }
        AiAction::Investigate {
            target,
            mode,
            monitor_seconds,
        } => {
            let spinner = ctx.ui.spinner("Investigating local runtime");
            let mut query = mode
                .as_ref()
                .map(|value| format!("?mode={value}"))
                .unwrap_or_default();
            if let Some(ms) = monitor_seconds {
                if query.is_empty() {
                    query.push('?');
                } else {
                    query.push('&');
                }
                query.push_str(&format!("monitor_seconds={ms}"));
            }
            let path = match target.unwrap_or(InvestigateTarget::Latest) {
                InvestigateTarget::Latest => format!("/api/investigate/now{query}"),
                InvestigateTarget::Incident { incident_id } => {
                    format!("/api/investigate/incident/{incident_id}{query}")
                }
                InvestigateTarget::Service { service_id } => {
                    format!("/api/investigate/service/{service_id}{query}")
                }
            };
            let payload = ctx.api_request(reqwest::Method::GET, &path, None).await?;
            spinner.finish("Investigation response ready");
            render_investigation(ctx, &payload);
            Ok(())
        }
    }
}

fn resolve_question(ctx: &AppContext, question: Option<String>) -> Result<String> {
    match question {
        Some(question) => Ok(question),
        None if ctx.ui.is_interactive() => {
            Ok(Input::new().with_prompt("Question").interact_text()?)
        }
        None => {
            bail!("question is required unless you run `inferra ai ask` in an interactive terminal")
        }
    }
}

fn render_ai_status(ctx: &AppContext, payload: &JsonValue) {
    if ctx.ui.is_json() {
        ctx.ui.print_json(payload);
        return;
    }
    ctx.ui
        .banner("AI status", "Configured provider and runtime availability");
    ctx.ui.kv_table([
        (
            "Enabled",
            payload["enabled"]
                .as_bool()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        ),
        (
            "Available",
            payload["available"]
                .as_bool()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        ),
        (
            "Provider",
            payload["provider"].as_str().unwrap_or("-").to_string(),
        ),
        (
            "Model",
            payload["model"].as_str().unwrap_or("-").to_string(),
        ),
        (
            "Base URL",
            payload["base_url"].as_str().unwrap_or("-").to_string(),
        ),
    ]);
    let guidance = payload["guidance"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !guidance.is_empty() {
        ctx.ui.paragraph("");
        ctx.ui.section("Guidance");
        ctx.ui.bullets(guidance);
    }
}

fn render_ai_doctor(ctx: &AppContext, payload: &JsonValue) {
    if ctx.ui.is_json() {
        ctx.ui.print_json(payload);
        return;
    }
    ctx.ui
        .banner("AI doctor", "Connectivity, model selection, and safeguards");
    ctx.ui.kv_table([
        (
            "Healthy",
            payload["ok"]
                .as_bool()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        ),
        (
            "Available",
            payload["available"]
                .as_bool()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        ),
        (
            "Provider",
            payload["provider"].as_str().unwrap_or("-").to_string(),
        ),
        (
            "Model",
            payload["model"].as_str().unwrap_or("-").to_string(),
        ),
        (
            "Investigate model",
            payload["investigate_model"]
                .as_str()
                .unwrap_or("-")
                .to_string(),
        ),
    ]);
    let warnings = payload["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !warnings.is_empty() {
        ctx.ui.paragraph("");
        ctx.ui.section("Warnings");
        ctx.ui.bullets(warnings);
    }
    let guidance = payload["guidance"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !guidance.is_empty() {
        ctx.ui.paragraph("");
        ctx.ui.section("Guidance");
        ctx.ui.bullets(guidance);
    }
}

fn render_investigation(ctx: &AppContext, payload: &JsonValue) {
    if ctx.ui.is_json() {
        ctx.ui.print_json(payload);
        return;
    }
    let output = &payload["output"];
    let headline = output["headline"]
        .as_str()
        .filter(|value| !value.is_empty())
        .unwrap_or("Investigation result");
    let subtitle = format!(
        "risk={} confidence={} used_ai={}",
        output["risk_level"].as_str().unwrap_or("unknown"),
        output["confidence"]
            .as_f64()
            .map(|value| format!("{:.0}%", value * 100.0))
            .unwrap_or_else(|| "-".to_string()),
        payload["used_ai"].as_bool().unwrap_or(false)
    );
    ctx.ui.banner(headline, &subtitle);

    if payload["used_ai"] == JsonValue::Bool(false) {
        let reason = payload["fallback_reason"]
            .as_str()
            .filter(|value| !value.is_empty())
            .unwrap_or("A deterministic fallback was used.");
        ctx.ui.warning(reason);
    }

    let warnings = payload["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !warnings.is_empty() {
        ctx.ui.paragraph("");
        ctx.ui.section("Warnings");
        ctx.ui.bullets(warnings);
    }

    render_text_section(ctx, "What happened", &output["what_happened"]);
    render_text_section(ctx, "Why it matters", &output["why_it_matters"]);
    render_text_section(ctx, "Likely causes", &output["likely_causes"]);

    let evidence_rows = output["evidence"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| {
            vec![
                item["type"].as_str().unwrap_or("-").to_string(),
                item["id"].as_str().unwrap_or("-").to_string(),
                item["summary"].as_str().unwrap_or_default().to_string(),
            ]
        })
        .collect::<Vec<_>>();
    if !evidence_rows.is_empty() {
        ctx.ui.paragraph("");
        ctx.ui.section("Evidence");
        ctx.ui.table(&["Type", "Id", "Summary"], evidence_rows);
    }

    let next_rows = output["next_steps"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| {
            vec![
                item["title"].as_str().unwrap_or_default().to_string(),
                item["reason"].as_str().unwrap_or("-").to_string(),
                item["command"].as_str().unwrap_or("-").to_string(),
            ]
        })
        .collect::<Vec<_>>();
    if !next_rows.is_empty() {
        ctx.ui.paragraph("");
        ctx.ui.section("Next steps");
        ctx.ui.table(&["Step", "Reason", "Command"], next_rows);
    }

    render_text_section(ctx, "Uncertainty", &output["uncertainty"]);
    render_text_section(ctx, "Missing evidence", &output["missing_evidence"]);

    let citations = output["citations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !citations.is_empty() {
        ctx.ui.paragraph("");
        ctx.ui.section("Citations");
        ctx.ui.bullets(citations);
    }
}

fn render_text_section(ctx: &AppContext, title: &str, value: &JsonValue) {
    let items = value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if items.is_empty() {
        return;
    }
    ctx.ui.paragraph("");
    ctx.ui.section(title);
    ctx.ui.bullets(items);
}

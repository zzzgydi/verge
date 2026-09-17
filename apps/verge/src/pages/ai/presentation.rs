use super::*;
use crate::ai::tools::Evidence;
use markdown::{ParseOptions, mdast::Node};

/// Keep Markdown layout without allowing model output to fetch images or embed HTML.
/// Parse source ranges so code examples and escaped punctuation remain untouched.
pub(super) fn safe_markdown(source: &str) -> String {
    fn visit(node: &Node, ranges: &mut Vec<std::ops::Range<usize>>) {
        if matches!(
            node,
            Node::Image(_) | Node::ImageReference(_) | Node::Html(_)
        ) {
            if let Some(p) = node.position() {
                ranges.push(p.start.offset..p.end.offset);
            }
        } else if let Some(children) = node.children() {
            for child in children {
                visit(child, ranges);
            }
        }
    }
    let Ok(tree) = markdown::to_mdast(source, &ParseOptions::gfm()) else {
        return source.replace('<', "&lt;").replace('!', "&#33;");
    };
    let mut ranges = Vec::new();
    visit(&tree, &mut ranges);
    let mut result = source.to_owned();
    for range in ranges.into_iter().rev() {
        result.replace_range(range, "[embedded content omitted]");
    }
    result
}

pub(super) fn status_text(lang: Lang, status: &str) -> String {
    let key = match status {
        "Preparing" => "ai.preparing",
        "Receiving response" => "ai.responding",
        "Completed" => "ai.complete",
        "Settings saved" => "ai.saved",
        "Testing provider" => "ai.testing",
        "Provider connection succeeded" => "ai.connected",
        "Cancelled" => "ai.cancelled",
        "Failed" => "ai.failed",
        s if s.starts_with("Reading ") => "ai.reading",
        s if s.starts_with("Waiting for model") => "ai.thinking",
        _ => "ai.preparing",
    };
    tr(lang, key).into()
}

pub(super) fn error_text(lang: Lang, error: &str) -> String {
    let key = if error == "Cancelled" {
        "ai.cancelled"
    } else if error.contains("401") || error.contains("403") {
        "ai.error_auth"
    } else if error.contains("429") {
        "ai.error_quota"
    } else if error.contains("404") {
        "ai.error_model"
    } else if error.contains("timed out") || error.contains("deadline") || error.contains("timeout")
    {
        "ai.error_network"
    } else if error.contains("Conversation limit") {
        "ai.error_limit"
    } else if error.contains("8 KiB") {
        "ai.too_long"
    } else if error.contains("URL")
        || error.contains("HTTPS")
        || error.contains("endpoint")
        || error.contains("model")
        || error.contains("timeout")
        || error.contains("rounds")
    {
        "ai.error_config"
    } else if error.contains("Keychain") || error.contains("key") || error.contains("credential") {
        "ai.error_key"
    } else {
        "ai.error_generic"
    };
    let friendly = tr(lang, key);
    friendly.into()
}

fn value(data: &serde_json::Value, key: &str) -> String {
    match &data[key] {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => "—".into(),
    }
}
fn yes(lang: Lang, value: &serde_json::Value) -> &'static str {
    tr(
        lang,
        if value.as_bool() == Some(true) {
            "ai.yes"
        } else {
            "ai.no"
        },
    )
}

fn summary(e: &Evidence, lang: Lang) -> String {
    let d = &e.data;
    if d["available"].as_bool() == Some(false) {
        return tr(lang, "ai.evidence_unavailable").into();
    }
    match e.source.as_str() {
        "runtime_status" => format!("{} · {}", value(d, "version"), value(d, "mode")),
        "system_proxy_status" => format!(
            "{}: {} · {}: {}",
            tr(lang, "ai.proxy_enabled"),
            yes(lang, &d["unified_enabled"]),
            tr(lang, "ai.recovery_pending"),
            yes(lang, &d["recovery_pending"])
        ),
        "proxy_summary" => format!("{}: {}", tr(lang, "ai.groups"), value(d, "total_groups")),
        "connection_summary" => format!(
            "{}: {} · ↑ {} B · ↓ {} B",
            tr(lang, "ai.connections"),
            value(d, "count"),
            value(d, "upload_bytes"),
            value(d, "download_bytes")
        ),
        "rule_summary" => format!("{}: {}", tr(lang, "ai.rules"), value(d, "total")),
        "recent_errors" => format!(
            "{}: {} · {}: {}",
            tr(lang, "ai.errors"),
            value(d, "error_count"),
            tr(lang, "ai.warnings"),
            value(d, "warning_count")
        ),
        "config_check" => format!(
            "{}: {} · {}: {}\n{}",
            tr(lang, "ai.selected"),
            yes(lang, &d["selected"]),
            tr(lang, "ai.merge_valid"),
            yes(lang, &d["merge_compilation_succeeded"]),
            tr(lang, "ai.config_scope")
        ),
        _ => tr(lang, "ai.evidence_unavailable").into(),
    }
}

pub(super) fn evidence_cards(evidence: &[Evidence], lang: Lang, cx: &App) -> Div {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut list = v_flex().gap_2();
    for e in evidence {
        let key = match e.source.as_str() {
            "runtime_status" => "ai.source_runtime",
            "system_proxy_status" => "ai.source_proxy",
            "proxy_summary" => "ai.source_nodes",
            "connection_summary" => "ai.source_connections",
            "rule_summary" => "ai.source_rules",
            "recent_errors" => "ai.source_errors",
            _ => "ai.source_config",
        };
        let age = now.saturating_sub(e.captured_at) / 60;
        let age = if age == 0 {
            tr(lang, "ai.just_now").into()
        } else {
            format!("{} {}", age, tr(lang, "ai.minutes_ago"))
        };
        let mut card = v_flex()
            .p_3()
            .gap_1()
            .rounded_lg()
            .bg(cx.theme().muted)
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .text_xs()
                    .child(format!("{} · {}", tr(lang, key), e.id))
                    .child(age),
            )
            .child(div().text_sm().whitespace_normal().child(summary(e, lang)));
        if let Some(groups) = e.data["groups"].as_array() {
            for group in groups.iter().take(12) {
                card = card.child(div().text_xs().child(format!(
                    "{} → {}",
                    value(group, "name"),
                    value(group, "selected")
                )));
            }
        }
        list = list.child(card);
    }
    list
}

#[cfg(test)]
mod tests {
    use super::{error_text, safe_markdown, summary};
    use crate::{ai::tools::Evidence, i18n::Lang};
    use markdown::{ParseOptions, mdast::Node};
    #[test]
    fn model_markdown_keeps_formatting_but_never_loads_embedded_content() {
        let text = "## Diagnosis\n\n**Check** `![code](url)`\n\n![inline](https://tracking.test/a)\n\n![ref][i]\n\n[i]: https://tracking.test/b\n\n<img src='file:///tmp/secret'>\n\n[help](https://example.com)";
        let safe = safe_markdown(text);
        assert!(safe.contains("**Check** `![code](url)`"));
        assert!(safe.contains("[help](https://example.com)"));
        let tree = markdown::to_mdast(&safe, &ParseOptions::gfm()).unwrap();
        fn has_image(n: &Node) -> bool {
            matches!(n, Node::Image(_) | Node::ImageReference(_) | Node::Html(_))
                || n.children().is_some_and(|c| c.iter().any(has_image))
        }
        assert!(!has_image(&tree));
    }
    #[test]
    fn errors_and_evidence_explain_what_to_do() {
        assert!(error_text(Lang::ZhCn, "AI provider HTTP 401").contains("密钥"));
        let e = Evidence {
            id: "R1-E7".into(),
            source: "config_check".into(),
            captured_at: 0,
            data: serde_json::json!({"selected":true,"merge_compilation_succeeded":true}),
        };
        assert!(summary(&e, Lang::ZhCn).contains("未重新运行"));
    }
}

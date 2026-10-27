//! One completion through the Copilot CLI: the summary, the title, and one step
//! of chat. Every call is its own SDK session with Humla's system prompt
//! replacing the CLI's agent prompt and no built-in tool exposed, so the model
//! is sent the prompt and nothing else. Chat's tools are declared without
//! handlers: the step ends at the assistant message that requests them, the
//! session is aborted before the CLI would run anything, and `chat::run_chat`
//! keeps driving the loop as it does for every provider.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use github_copilot_sdk::handler::DenyAllHandler;
use github_copilot_sdk::session_events::{AssistantMessageData, AssistantMessageToolRequest};
use github_copilot_sdk::rpc::ModelPolicyState;
use github_copilot_sdk::types::{SessionConfig, SystemMessageConfig};
use github_copilot_sdk::{CliProgram, Client, ClientOptions, Tool};

use crate::chat::adapter::{CancelFlag, ChatStep, ChatTurn, ToolCall, ToolSpec};
use crate::openai::StreamChunk;

/// What every model call through Copilot may take before it is given up on.
const CALL_TIMEOUT: Duration = Duration::from_secs(600);

/// The running CLI, kept between calls: starting one takes a second or two.
/// A new one starts when the binary or the token changes; the old one lives on
/// until the calls still holding it finish.
#[derive(Default)]
pub struct Runtime {
    running: tokio::sync::Mutex<Option<Running>>,
    pub(super) cli_path: tokio::sync::Mutex<Option<PathBuf>>,
    /// Held across a token refresh: GitHub rotates the refresh token, so a
    /// second refresh with the same one is refused and would sign the user out.
    pub(super) refreshing: tokio::sync::Mutex<()>,
}

struct Running {
    cli: PathBuf,
    token: String,
    client: Client,
}

impl Runtime {
    pub async fn client(&self, cli: &Path, token: &str) -> anyhow::Result<Client> {
        let mut running = self.running.lock().await;
        if let Some(r) = running.as_ref() {
            if r.cli == cli && r.token == token && r.client.ping(None).await.is_ok() {
                return Ok(r.client.clone());
            }
        }
        let mut opts = ClientOptions::default();
        opts.program = CliProgram::Path(cli.to_path_buf());
        opts.github_token = Some(token.to_string());
        opts.use_logged_in_user = Some(false);
        opts.working_directory = std::env::temp_dir();
        opts.extra_args = vec![
            // The SDK refuses a CLI outside its protocol range, so the CLI
            // must not swap itself for a newer download.
            "--no-auto-update".into(),
            "--disable-builtin-mcps".into(),
            "--disable-client-telemetry".into(),
        ];
        let client = Client::start(opts).await.map_err(|e| {
            anyhow!(
                "Couldn't start the Copilot CLI: {e}. If it is an older or newer version than \
                 {}, download the one Humla supports in Settings.",
                super::cli::VERSION
            )
        })?;
        *running = Some(Running { cli: cli.to_path_buf(), token: token.to_string(), client: client.clone() });
        Ok(client)
    }

    pub async fn stop(&self) {
        if let Some(r) = self.running.lock().await.take() {
            let _ = r.client.stop().await;
        }
    }
}

/// A chat-capable model the account may use, for the Settings picker.
#[derive(serde::Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub vendor: Option<String>,
    /// Premium requests per call, relative to the base rate; 0 is included.
    pub multiplier: Option<f64>,
}

pub async fn list_models(client: &Client) -> anyhow::Result<Vec<ModelInfo>> {
    let models = client.list_models().await.map_err(|e| anyhow!("{e}"))?;
    Ok(models
        .into_iter()
        .filter(|m| m.policy.as_ref().is_none_or(|p| p.state == ModelPolicyState::Enabled))
        .map(|m| ModelInfo {
            multiplier: m.billing.as_ref().and_then(|b| b.multiplier),
            id: m.id,
            name: m.name,
            vendor: m.vendor,
        })
        .collect())
}

/// The model that costs the least per call, for work the user didn't ask
/// for: the automatic title. `auto` is never it, since it may route anywhere.
pub fn cheapest(models: &[ModelInfo]) -> Option<&str> {
    models
        .iter()
        .filter(|m| m.id != "auto")
        .filter_map(|m| m.multiplier.map(|x| (x, m.id.as_str())))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id)
}

/// The single prompt a step sends: the system turns become the system prompt,
/// and everything after them is written out as one transcript, since a
/// session carries no history Humla didn't put in this message.
pub fn render_prompt(turns: &[ChatTurn]) -> (String, String) {
    let system = turns
        .iter()
        .filter(|t| t.role == "system")
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let rest: Vec<&ChatTurn> = turns.iter().filter(|t| t.role != "system").collect();
    if let [only] = rest.as_slice() {
        if only.role == "user" {
            return (system, only.text.clone());
        }
    }
    let mut out = String::from(
        "The conversation so far, oldest first. Continue it: answer the last user \
         message, or call a tool.\n",
    );
    for turn in rest {
        match turn.role.as_str() {
            "tool" => out.push_str(&format!(
                "\n<tool_result call=\"{}\">\n{}\n</tool_result>\n",
                turn.tool_call_id.as_deref().unwrap_or_default(),
                turn.text
            )),
            role => {
                out.push_str(&format!("\n<{role}>\n"));
                if !turn.text.is_empty() {
                    out.push_str(&turn.text);
                    out.push('\n');
                }
                for call in &turn.tool_calls {
                    out.push_str(&format!(
                        "[called {} {} as {}]\n",
                        call.name, call.arguments, call.id
                    ));
                }
                out.push_str(&format!("</{role}>\n"));
            }
        }
    }
    (system, out)
}

/// The step an assistant message amounts to.
pub fn step_from_message(content: &str, tool_requests: &[AssistantMessageToolRequest]) -> ChatStep {
    ChatStep {
        text: content.to_string(),
        tool_calls: tool_requests
            .iter()
            .map(|r| ToolCall {
                id: r.tool_call_id.clone(),
                name: r.name.clone(),
                arguments: r
                    .arguments
                    .as_ref()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "{}".into()),
            })
            .collect(),
    }
}

/// What one session streams, as it arrives.
enum Delta<'a> {
    Content(&'a str),
    Thinking(&'a str),
}

/// Run one session to its first assistant message that requests tools, or to
/// its end. `on_delta` returns `false` to stop early.
async fn run_once(
    client: &Client,
    model: &str,
    system: &str,
    prompt: &str,
    tools: Vec<Tool>,
    mut on_delta: impl FnMut(Delta<'_>) -> bool + Send,
) -> anyhow::Result<ChatStep> {
    let mut cfg = SessionConfig::default().with_permission_handler(Arc::new(DenyAllHandler));
    cfg.streaming = Some(true);
    cfg.model = Some(model.to_string());
    cfg.system_message =
        Some(SystemMessageConfig::new().with_mode("replace").with_content(system));
    cfg.available_tools = Some(tools.iter().map(|t| t.name.clone()).collect());
    cfg.tools = (!tools.is_empty()).then_some(tools);

    let session = client
        .create_session(cfg)
        .await
        .map_err(|e| anyhow!("Copilot couldn't start a session: {e}"))?;
    let mut events = session.subscribe();
    let id = session.id().clone();

    let outcome = async {
        session.send(prompt).await.map_err(|e| anyhow!("Copilot refused the message: {e}"))?;
        let mut text = String::new();
        loop {
            let ev = events.recv().await.map_err(|e| anyhow!("Copilot stopped answering: {e:?}"))?;
            let data = &ev.data;
            match ev.event_type.as_str() {
                "assistant.message_delta" | "assistant.reasoning_delta" => {
                    let delta = data.get("deltaContent").and_then(|v| v.as_str()).unwrap_or_default();
                    let keep_going = if ev.event_type == "assistant.message_delta" {
                        text.push_str(delta);
                        on_delta(Delta::Content(delta))
                    } else {
                        on_delta(Delta::Thinking(delta))
                    };
                    if !keep_going {
                        return Ok(ChatStep { text, tool_calls: Vec::new() });
                    }
                }
                "assistant.message" => {
                    let msg: AssistantMessageData =
                        ev.typed_data().context("Copilot sent a message Humla can't read")?;
                    let requests = msg.tool_requests.unwrap_or_default();
                    if !requests.is_empty() {
                        return Ok(step_from_message(&msg.content, &requests));
                    }
                    text = msg.content;
                }
                "session.error" => {
                    let message = data.get("message").and_then(|v| v.as_str()).unwrap_or("unknown error");
                    bail!("Copilot: {message}");
                }
                "session.idle" => return Ok(ChatStep { text, tool_calls: Vec::new() }),
                _ => {}
            }
        }
    };
    let result = tokio::time::timeout(CALL_TIMEOUT, outcome)
        .await
        .unwrap_or_else(|_| Err(anyhow!("Copilot took too long to answer.")));

    // A tool request leaves the session waiting to run it; nothing here
    // should, and the session has no further use.
    let _ = session.abort().await;
    let _ = session.disconnect().await;
    let _ = client.delete_session(&id).await;
    result
}

/// A summary or a title.
pub async fn complete(
    client: &Client,
    model: &str,
    system: &str,
    user: &str,
    mut on_chunk: impl FnMut(StreamChunk) + Send,
) -> anyhow::Result<String> {
    let step = run_once(client, model, system, user, Vec::new(), |d| {
        match d {
            Delta::Content(s) => on_chunk(StreamChunk::Content(s)),
            Delta::Thinking(s) => on_chunk(StreamChunk::Thinking(s)),
        }
        true
    })
    .await?;
    if step.text.trim().is_empty() {
        bail!("Copilot returned an empty answer.");
    }
    Ok(step.text)
}

/// One step of chat.
pub async fn step(
    client: &Client,
    model: &str,
    turns: &[ChatTurn],
    tools: &[ToolSpec],
    cancel: &CancelFlag,
    mut on_text: impl FnMut(&str) + Send,
) -> anyhow::Result<ChatStep> {
    let (system, prompt) = render_prompt(turns);
    let tools = tools
        .iter()
        .map(|t| Tool::new(t.name).with_description(t.description).with_parameters(t.parameters.clone()))
        .collect();
    run_once(client, model, &system, &prompt, tools, |d| {
        if let Delta::Content(s) = d {
            on_text(s);
        }
        !cancel.is_cancelled()
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model(id: &str, multiplier: Option<f64>) -> ModelInfo {
        ModelInfo { id: id.into(), name: id.into(), vendor: None, multiplier }
    }

    #[test]
    fn the_cheapest_model_is_the_lowest_multiplier_and_never_auto() {
        let models = [
            model("auto", Some(0.0)),
            model("claude-opus-5.5", Some(10.0)),
            model("gpt-5-mini", Some(0.0)),
            model("claude-haiku-5.5", Some(0.33)),
            model("grok-4.7", None),
        ];
        assert_eq!(cheapest(&models), Some("gpt-5-mini"));
        assert_eq!(cheapest(&[model("auto", Some(0.0)), model("x", None)]), None);
    }

    #[test]
    fn a_first_turn_goes_as_it_is() {
        let (system, prompt) = render_prompt(&[
            ChatTurn::new("system", "You answer from notes."),
            ChatTurn::new("system", "Today is 27 Oct."),
            ChatTurn::new("user", "What did Acme decide?"),
        ]);
        assert_eq!(system, "You answer from notes.\n\nToday is 27 Oct.");
        assert_eq!(prompt, "What did Acme decide?");
    }

    #[test]
    fn a_tool_trail_is_written_out_in_order() {
        let call = ToolCall { id: "c1".into(), name: "search_notes".into(), arguments: r#"{"query":"Acme"}"#.into() };
        let (system, prompt) = render_prompt(&[
            ChatTurn::new("system", "S"),
            ChatTurn::new("user", "What did Acme decide?"),
            ChatTurn::assistant_tool_calls("", vec![call]),
            ChatTurn::tool_result("c1", "Acme kept the budget."),
        ]);
        assert_eq!(system, "S");
        assert_eq!(
            prompt,
            "The conversation so far, oldest first. Continue it: answer the last user message, \
             or call a tool.\n\
             \n<user>\nWhat did Acme decide?\n</user>\n\
             \n<assistant>\n[called search_notes {\"query\":\"Acme\"} as c1]\n</assistant>\n\
             \n<tool_result call=\"c1\">\nAcme kept the budget.\n</tool_result>\n"
        );
    }

    #[test]
    fn a_grounded_follow_up_keeps_every_turn() {
        let (_, prompt) = render_prompt(&[
            ChatTurn::new("system", "S"),
            ChatTurn::new("user", "Note: the Acme meeting."),
            ChatTurn::new("user", "Summarise it"),
            ChatTurn::new("assistant", "They kept the budget."),
            ChatTurn::new("user", "And the date?"),
        ]);
        assert!(prompt.contains("<user>\nNote: the Acme meeting.\n</user>"));
        assert!(prompt.contains("<assistant>\nThey kept the budget.\n</assistant>"));
        assert!(prompt.trim_end().ends_with("<user>\nAnd the date?\n</user>"));
    }

    /// Against the real CLI and account; spends Copilot requests. Run with
    /// `HUMLA_COPILOT_TOKEN=$(gh auth token) cargo test --lib copilot_live -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn copilot_live() {
        let token = std::env::var("HUMLA_COPILOT_TOKEN").expect("HUMLA_COPILOT_TOKEN");
        let cli = super::super::cli::locate(&std::env::temp_dir()).await.expect("a Copilot CLI");
        let rt = Runtime::default();
        let client = rt.client(&cli, &token).await.unwrap();

        let models = list_models(&client).await.unwrap();
        assert!(models.iter().any(|m| m.id == "auto"), "{models:?}");

        let mut streamed = String::new();
        let text = complete(&client, "auto", "Answer with one word.", "Say hello in Norwegian.", |c| {
            if let StreamChunk::Content(s) = c {
                streamed.push_str(s);
            }
        })
        .await
        .unwrap();
        assert!(!text.trim().is_empty());
        assert_eq!(streamed, text);

        let tool = ToolSpec {
            name: "search_notes",
            description: "Search the user's meeting notes by keyword.",
            parameters: json!({ "type": "object", "properties": { "query": { "type": "string" } }, "required": ["query"] }),
        };
        let turns = [
            ChatTurn::new("system", "You answer only from the user's notes. Always search first."),
            ChatTurn::new("user", "What did Acme decide about the budget?"),
        ];
        let first = step(&client, "auto", &turns, &[tool.clone()], &CancelFlag::new(), |_| {}).await.unwrap();
        assert!(!first.tool_calls.is_empty(), "{first:?}");
        assert_eq!(first.tool_calls[0].name, "search_notes");

        let mut followed = turns.to_vec();
        followed.push(ChatTurn::assistant_tool_calls("", first.tool_calls.clone()));
        for call in &first.tool_calls {
            followed.push(ChatTurn::tool_result(&call.id, "Acme kept the budget at 2 MNOK (meeting 3 Oct)."));
        }
        let answer = step(&client, "auto", &followed, &[tool], &CancelFlag::new(), |_| {}).await.unwrap();
        assert!(answer.tool_calls.is_empty(), "{answer:?}");
        assert!(answer.text.contains('2'), "{}", answer.text);
        rt.stop().await;
    }

    #[test]
    fn tool_requests_become_tool_calls_with_their_arguments_as_json() {
        let requests: Vec<AssistantMessageToolRequest> = serde_json::from_value(json!([
            { "toolCallId": "toolu_1", "name": "search_notes", "arguments": { "query": "Acme budsjett" }, "type": "function" },
            { "toolCallId": "toolu_2", "name": "list_notes", "type": "function" }
        ]))
        .unwrap();
        assert_eq!(
            step_from_message("", &requests),
            ChatStep {
                text: String::new(),
                tool_calls: vec![
                    ToolCall { id: "toolu_1".into(), name: "search_notes".into(), arguments: r#"{"query":"Acme budsjett"}"#.into() },
                    ToolCall { id: "toolu_2".into(), name: "list_notes".into(), arguments: "{}".into() },
                ],
            }
        );
    }
}

//! MCP Tool Argument Character Limit Policy — entrypoint.
//!
//! This is a *pass-through* policy for every MCP method except `tools/call`.
//! For `tools/call` it enforces per-field and total character limits on the
//! `arguments` object, returning `-32602 Invalid params` before any upstream
//! sees the request when a limit is violated.
//!
//! For all other MCP methods (initialize, tools/list, ping, notifications)
//! the policy passes through so they reach the next policy or the actual MCP
//! server.  This means it can sit in front of **any** MCP backend — it does
//! not implement the full MCP server itself.
//!
//! Request flow for tools/call:
//!   1. Buffer headers + body atomically (into_headers_body_state, #15).
//!   2. Enforce maxRequestBytes.
//!   3. Parse the JSON-RPC envelope.
//!   4. Extract tools/call arguments.
//!   5. Run per-field and total character-limit checks (argcheck module).
//!   6a. REJECT: return -32602 Invalid params — no upstream reached.
//!   6b. PASS: Flow::Continue — the request proceeds downstream unchanged.

mod argcheck;
mod config;
mod generated;
mod jsonrpc;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use pdk::hl::*;
use pdk::logger;
use serde_json::{json, Value};

use crate::config::PolicyConfig;
use crate::generated::config::Config;
use crate::jsonrpc::{
    error_response, JsonRpcOutbound, JsonRpcRequest, INVALID_PARAMS, INVALID_REQUEST, PARSE_ERROR,
};

const POLICY_NAME: &str = "mcp-maxchar-length";
const CONTENT_TYPE_HEADER: &str = "content-type";
const CONTENT_LENGTH_HEADER: &str = "content-length";
const APPLICATION_JSON: &str = "application/json";

// MCP protocol versions this policy understands.
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
const DEFAULT_NEGOTIATED_VERSION: &str = "2025-03-26";

fn is_supported_version(version: &str) -> bool {
    SUPPORTED_PROTOCOL_VERSIONS.contains(&version)
}

// ---------------------------------------------------------------------------
// Request filter
// ---------------------------------------------------------------------------

async fn request_filter(
    request: RequestState,
    policy: Rc<PolicyConfig>,
) -> Flow<()> {
    // Read path and method via into_headers_state() — this does NOT enable
    // stop_iteration, so GET/SSE requests are never paused by Envoy.
    // into_headers_body_state() (which does enable stop_iteration) is only
    // called for POST requests that actually carry a JSON-RPC body.
    let headers_state = request.into_headers_state().await;
    let path = headers_state.path();
    let method = headers_state.method().to_ascii_uppercase();

    // Strict-mode: non-MCP paths fall through or 404.
    let bare = path.split_once('?').map(|(p, _)| p).unwrap_or(&path);
    if !bare.starts_with(&policy.mcp_endpoint) {
        if policy.strict_mode {
            return send_error(404, "Not Found");
        }
        return Flow::Continue(());
    }

    // Only POST carries MCP JSON-RPC messages — GET/DELETE/etc. (SSE handshake,
    // session teardown) must pass through untouched to mcp-support-policy.
    if method != "POST" {
        return Flow::Continue(());
    }

    // Now safe to buffer: POST only. Atomic header+body buffering ensures our
    // Flow::Break response cannot race an upstream response (#15).
    let state = headers_state.into_headers_body_state().await;
    let handler = state.handler();

    // Content-Type must be application/json for POST.
    let content_type = handler.header("content-type").unwrap_or_default();
    if !content_type.contains("application/json") {
        return send_json_rpc(
            200,
            &error_response(
                None,
                INVALID_REQUEST,
                "Content-Type must be application/json",
            ),
        );
    }

    // MCP-Protocol-Version header: validated here so transport violations are
    // caught before touching the body.  `initialize` is exempt.
    let requested_protocol_version: Option<String> = handler.header("mcp-protocol-version");

    let body = if state.contains_body() {
        handler.body()
    } else {
        Vec::new()
    };

    // Payload-size cap: reject an oversized request body before parsing.
    if body.len() > policy.max_request_bytes {
        return send_json_rpc(
            200,
            &error_response(
                None,
                INVALID_REQUEST,
                format!(
                    "request body of {} bytes exceeds limit of {} bytes",
                    body.len(),
                    policy.max_request_bytes
                ),
            ),
        );
    }

    let rpc: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return send_json_rpc(
                200,
                &error_response(None, PARSE_ERROR, format!("Parse error: {e}")),
            );
        }
    };

    if rpc.jsonrpc.as_deref() != Some("2.0") {
        return send_json_rpc(
            200,
            &error_response(rpc.id, INVALID_REQUEST, r#"jsonrpc must be "2.0""#),
        );
    }

    let method_name = match &rpc.method {
        Some(m) => m.clone(),
        None => {
            return send_json_rpc(
                200,
                &error_response(rpc.id, INVALID_REQUEST, "missing method"),
            );
        }
    };

    // MCP-Protocol-Version header check (exempt for initialize).
    if method_name != "initialize" {
        if let Some(pv) = requested_protocol_version.as_deref() {
            if !is_supported_version(pv) {
                return send_json_rpc(
                    400,
                    &error_response(
                        rpc.id.clone(),
                        INVALID_REQUEST,
                        format!("unsupported MCP-Protocol-Version '{pv}'"),
                    ),
                );
            }
        } else {
            logger::debug!(
                "[{}] no MCP-Protocol-Version header; assuming {}",
                POLICY_NAME,
                DEFAULT_NEGOTIATED_VERSION
            );
        }
    }

    // Notifications (id-less): pass through — they carry no arguments.
    if rpc.is_notification() {
        return Flow::Continue(());
    }

    // For tools/call: enforce character limits, then continue.
    // For everything else: continue immediately.
    if method_name == "tools/call" {
        if let Err(err_msg) =
            check_tools_call_args(rpc.id.clone(), rpc.params.as_ref(), &policy)
        {
            return err_msg;
        }
    }

    // All checks passed — let the request proceed downstream.
    Flow::Continue(())
}

// ---------------------------------------------------------------------------
// Argument character-limit check for tools/call
// ---------------------------------------------------------------------------

/// Validate the arguments in a `tools/call` params object against the
/// configured character limits.  Returns `Ok(())` when all limits are
/// satisfied, or `Err(Flow)` containing the rejection response.
fn check_tools_call_args(
    id: Option<Value>,
    params: Option<&Value>,
    policy: &PolicyConfig,
) -> Result<(), Flow<()>> {
    let params = params.cloned().unwrap_or(json!({}));

    // arguments must be an object when present.
    let raw_args = match params.get("arguments") {
        Some(Value::Object(_)) | None => {
            params.get("arguments").cloned().unwrap_or(json!({}))
        }
        Some(other) => {
            return Err(send_json_rpc(
                200,
                &error_response(
                    id,
                    INVALID_PARAMS,
                    format!("'arguments' must be a JSON object, got {}", other),
                ),
            ));
        }
    };

    // Run the character-limit checks.
    if let Err(msg) = argcheck::check_argument_chars(&raw_args, policy) {
        logger::info!(
            "[{}] tools/call rejected: {}",
            POLICY_NAME,
            msg
        );
        return Err(send_json_rpc(
            200,
            &error_response(
                id,
                INVALID_PARAMS,
                format!("argument character limit exceeded — {msg}"),
            ),
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

fn send_json_rpc(status: u32, payload: &JsonRpcOutbound) -> Flow<()> {
    let body = serde_json::to_vec(payload).unwrap_or_default();
    send_raw(status, &[(CONTENT_TYPE_HEADER, APPLICATION_JSON)], &body)
}

fn send_error(status: u32, detail: &str) -> Flow<()> {
    let body = format!(r#"{{"error":"{}"}}"#, detail);
    send_raw(
        status,
        &[(CONTENT_TYPE_HEADER, APPLICATION_JSON)],
        body.as_bytes(),
    )
}

fn send_raw(status: u32, headers: &[(&str, &str)], body: &[u8]) -> Flow<()> {
    let mut owned: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    owned.push((CONTENT_LENGTH_HEADER.to_string(), body.len().to_string()));
    Flow::Break(Response::new(status).with_headers(owned).with_body(body))
}

// ---------------------------------------------------------------------------
// PDK entrypoint
// ---------------------------------------------------------------------------

#[entrypoint]
pub async fn configure(
    launcher: Launcher,
    Configuration(bytes): Configuration,
) -> anyhow::Result<()> {
    let raw: Config = serde_json::from_slice(&bytes)
        .map_err(|e| anyhow::anyhow!("invalid policy configuration: {e}"))?;

    let policy = PolicyConfig::from_config(&raw)
        .map_err(|e| anyhow::anyhow!("policy configuration rejected: {e}"))?;

    logger::info!(
        "[{}] loaded; endpoint='{}'; default_max={:?}; field_limits={}; total_max={:?}; req_limit={}B",
        POLICY_NAME,
        policy.mcp_endpoint,
        policy.default_field_max_chars,
        policy.field_limits.len(),
        policy.max_total_argument_chars,
        policy.max_request_bytes,
    );

    let policy = Rc::new(policy);

    let filter = on_request(move |request: RequestState, _client: HttpClient| {
        let policy = policy.clone();
        async move { request_filter(request, policy).await }
    });

    launcher.launch(filter).await?;
    Ok(())
}

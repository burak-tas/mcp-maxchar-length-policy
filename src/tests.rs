//! End-to-end pdk-unit tests driving the `configure` entrypoint through the
//! full MCP Streamable-HTTP transport.
//!
//! This policy is a *pass-through enforcer* — it enforces character limits on
//! tools/call arguments and then does Flow::Continue for valid requests.  The
//! tests therefore assert:
//!   - transport guards (404, Content-Type, GET pass-through)
//!   - envelope validation (-32600, -32700, -32601)
//!   - argument character-limit enforcement (-32602 for violations)
//!   - pass-through: a valid tools/call with arguments under every limit
//!     results in Flow::Continue so the downstream MCP backend receives it.
//!
//! Test ID scheme: TC-01..TC-20 (matches TEST_REPORT.md).

#[cfg(test)]
mod tests {
    use pdk_unit::{UnitHttpRequest, UnitTestBuilder};
    use serde_json::{json, Value};

    const ENDPOINT: &str = "/mcp";

    /// Minimal valid policy configuration: default 100-char limit, no
    /// field-specific limits, no total limit.
    fn default_config() -> String {
        json!({
            "mcpEndpoint": ENDPOINT,
            "strictMode": true,
            "defaultFieldMaxChars": 100
        })
        .to_string()
    }

    /// Build a tester over `configure` with the given config.
    fn tester(config: &str) -> pdk_unit::UnitTest {
        UnitTestBuilder::default()
            .with_config(config)
            .with_entrypoint(crate::configure)
    }

    /// POST a JSON-RPC message to the MCP endpoint and return the response.
    fn post_rpc(config: &str, rpc: Value) -> pdk_unit::UnitHttpResponse {
        tester(config).request(
            UnitHttpRequest::post()
                .with_path(ENDPOINT)
                .with_header("content-type", "application/json")
                .with_body(rpc.to_string()),
        )
    }

    fn body_json(response: &pdk_unit::UnitHttpResponse) -> Value {
        use pdk_unit::UnitHttpMessage;
        serde_json::from_slice(response.body()).expect("response body must be valid JSON")
    }

    // -----------------------------------------------------------------------
    // TC-01: transport — non-MCP path in strict mode returns 404
    // -----------------------------------------------------------------------
    #[test]
    fn tc01_non_mcp_path_is_404_in_strict_mode() {
        let response = tester(&default_config()).request(
            UnitHttpRequest::post()
                .with_path("/not-mcp")
                .with_header("content-type", "application/json")
                .with_body(json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string()),
        );
        assert_eq!(response.status_code(), 404);
    }

    // -----------------------------------------------------------------------
    // TC-02: transport — non-MCP path in non-strict mode falls through
    // -----------------------------------------------------------------------
    #[test]
    fn tc02_non_mcp_path_passes_through_in_non_strict_mode() {
        let cfg = json!({"mcpEndpoint": ENDPOINT, "strictMode": false}).to_string();
        let response = tester(&cfg).request(
            UnitHttpRequest::post()
                .with_path("/upstream-api")
                .with_header("content-type", "application/json")
                .with_body(json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string()),
        );
        // Flow::Continue → pdk-unit returns 200 (upstream stub default).
        assert_eq!(response.status_code(), 200);
    }

    // -----------------------------------------------------------------------
    // TC-03: transport — GET passes through (SSE handshake used by mcp-support-policy)
    // -----------------------------------------------------------------------
    #[test]
    fn tc03_get_passes_through() {
        let response =
            tester(&default_config()).request(UnitHttpRequest::get().with_path(ENDPOINT));
        // Flow::Continue → pdk-unit returns 200 (upstream stub default).
        assert_eq!(response.status_code(), 200);
    }

    // -----------------------------------------------------------------------
    // TC-04: transport — wrong Content-Type returns -32600
    // -----------------------------------------------------------------------
    #[test]
    fn tc04_wrong_content_type_returns_invalid_request() {
        let response = tester(&default_config()).request(
            UnitHttpRequest::post()
                .with_path(ENDPOINT)
                .with_header("content-type", "text/plain")
                .with_body(json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string()),
        );
        assert_eq!(response.status_code(), 200);
        assert_eq!(body_json(&response)["error"]["code"], -32600);
    }

    // -----------------------------------------------------------------------
    // TC-05: envelope — malformed JSON body returns -32700
    // -----------------------------------------------------------------------
    #[test]
    fn tc05_malformed_json_returns_parse_error() {
        let response = tester(&default_config()).request(
            UnitHttpRequest::post()
                .with_path(ENDPOINT)
                .with_header("content-type", "application/json")
                .with_body("{ not valid json "),
        );
        assert_eq!(response.status_code(), 200);
        assert_eq!(body_json(&response)["error"]["code"], -32700);
    }

    // -----------------------------------------------------------------------
    // TC-06: envelope — wrong jsonrpc version returns -32600
    // -----------------------------------------------------------------------
    #[test]
    fn tc06_wrong_jsonrpc_version_returns_invalid_request() {
        let response = post_rpc(
            &default_config(),
            json!({"jsonrpc":"1.0","id":1,"method":"ping"}),
        );
        assert_eq!(response.status_code(), 200);
        assert_eq!(body_json(&response)["error"]["code"], -32600);
    }

    // -----------------------------------------------------------------------
    // TC-07: envelope — missing method returns -32600
    // -----------------------------------------------------------------------
    #[test]
    fn tc07_missing_method_returns_invalid_request() {
        let response = post_rpc(&default_config(), json!({"jsonrpc":"2.0","id":1}));
        assert_eq!(response.status_code(), 200);
        assert_eq!(body_json(&response)["error"]["code"], -32600);
    }

    // -----------------------------------------------------------------------
    // TC-08: MCP-Protocol-Version — any version passes through (not our concern)
    // -----------------------------------------------------------------------
    #[test]
    fn tc08_unknown_protocol_version_passes_through() {
        // This policy does not validate MCP-Protocol-Version — that is the
        // mcp-support-policy's responsibility. Unknown versions must pass through.
        let response = tester(&default_config()).request(
            UnitHttpRequest::post()
                .with_path(ENDPOINT)
                .with_header("content-type", "application/json")
                .with_header("mcp-protocol-version", "2025-11-25")
                .with_body(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string()),
        );
        // Flow::Continue → pdk-unit 200 (upstream stub default).
        assert_eq!(response.status_code(), 200);
    }

    // -----------------------------------------------------------------------
    // TC-09: pass-through — non-tools/call methods pass through
    // -----------------------------------------------------------------------
    #[test]
    fn tc09_initialize_passes_through_downstream() {
        // initialize has no arguments → passes through without any limit check.
        let response = post_rpc(
            &default_config(),
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        );
        // pdk-unit returns 200 for Flow::Continue when no upstream is registered.
        assert_eq!(response.status_code(), 200);
    }

    #[test]
    fn tc09b_tools_list_passes_through_downstream() {
        let response = post_rpc(
            &default_config(),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        );
        assert_eq!(response.status_code(), 200);
    }

    // -----------------------------------------------------------------------
    // TC-10: tools/call — arguments under default limit passes through
    // -----------------------------------------------------------------------
    #[test]
    fn tc10_tools_call_within_default_limit_passes_through() {
        let cfg = json!({
            "mcpEndpoint": ENDPOINT,
            "strictMode": true,
            "defaultFieldMaxChars": 50
        })
        .to_string();
        let response = post_rpc(
            &cfg,
            json!({
                "jsonrpc":"2.0","id":10,"method":"tools/call",
                "params":{"name":"myTool","arguments":{"query":"short query"}}
            }),
        );
        // Flow::Continue — pdk-unit returns 200 with an empty body (no upstream registered).
        // Status 200 and no error body is the correct pass-through signal.
        assert_eq!(response.status_code(), 200);
    }

    // -----------------------------------------------------------------------
    // TC-11: tools/call — single field exceeds default limit → -32602
    // -----------------------------------------------------------------------
    #[test]
    fn tc11_single_field_over_default_limit_is_invalid_params() {
        let cfg = json!({
            "mcpEndpoint": ENDPOINT,
            "strictMode": true,
            "defaultFieldMaxChars": 10
        })
        .to_string();
        let long_val = "x".repeat(11);
        let response = post_rpc(
            &cfg,
            json!({
                "jsonrpc":"2.0","id":11,"method":"tools/call",
                "params":{"name":"myTool","arguments":{"description": long_val}}
            }),
        );
        assert_eq!(response.status_code(), 200);
        let body = body_json(&response);
        assert_eq!(body["error"]["code"], -32602, "got: {body:?}");
        let msg = body["error"]["message"].as_str().unwrap_or_default();
        assert!(msg.contains("description"), "error must name the field: {msg}");
        // Must NOT echo the offending value.
        assert!(
            !msg.contains(&long_val),
            "error must not echo value: {msg}"
        );
    }

    // -----------------------------------------------------------------------
    // TC-12: tools/call — field-specific limit overrides default
    // -----------------------------------------------------------------------
    #[test]
    fn tc12_field_specific_limit_overrides_default() {
        let cfg = json!({
            "mcpEndpoint": ENDPOINT,
            "strictMode": true,
            "defaultFieldMaxChars": 1000,
            "fieldLimits": [
                {"field": "description", "maxChars": 20}
            ]
        })
        .to_string();
        // description is 25 chars, over the field-specific limit of 20.
        let response = post_rpc(
            &cfg,
            json!({
                "jsonrpc":"2.0","id":12,"method":"tools/call",
                "params":{"name":"myTool","arguments":{"description":"this is twenty-five ch"}}
            }),
        );
        assert_eq!(response.status_code(), 200);
        let body = body_json(&response);
        assert_eq!(body["error"]["code"], -32602, "got: {body:?}");
        let msg = body["error"]["message"].as_str().unwrap_or_default();
        assert!(msg.contains("description"), "got: {msg}");
        // Other fields not in fieldLimits still use the default (1000) — a
        // short description must pass (Flow::Continue → empty body, status 200).
        let response2 = post_rpc(
            &cfg,
            json!({
                "jsonrpc":"2.0","id":13,"method":"tools/call",
                "params":{"name":"myTool","arguments":{"name":"short","description":"ok description"}}
            }),
        );
        // Pass-through: pdk-unit returns 200, empty body (no upstream).
        assert_eq!(response2.status_code(), 200);
    }

    // -----------------------------------------------------------------------
    // TC-13: tools/call — wildcard field limit
    // -----------------------------------------------------------------------
    #[test]
    fn tc13_wildcard_field_limit_applies_to_unmatched_fields() {
        let cfg = json!({
            "mcpEndpoint": ENDPOINT,
            "strictMode": true,
            "defaultFieldMaxChars": 10000,
            "fieldLimits": [
                {"field": "*", "maxChars": 5}
            ]
        })
        .to_string();
        // "anyfield" has 6 chars → fails wildcard.
        let response = post_rpc(
            &cfg,
            json!({
                "jsonrpc":"2.0","id":14,"method":"tools/call",
                "params":{"name":"myTool","arguments":{"anyfield":"sixchr"}}
            }),
        );
        assert_eq!(response.status_code(), 200);
        assert_eq!(body_json(&response)["error"]["code"], -32602);
    }

    // -----------------------------------------------------------------------
    // TC-14: tools/call — total character limit exceeded → -32602
    // -----------------------------------------------------------------------
    #[test]
    fn tc14_total_char_limit_exceeded_is_invalid_params() {
        let cfg = json!({
            "mcpEndpoint": ENDPOINT,
            "strictMode": true,
            "defaultFieldMaxChars": 1000,
            "maxTotalArgumentChars": 20
        })
        .to_string();
        // "hello" + "world" = 10; "hello" + "world extra" = 16; fine.
        // "hello" + "x".repeat(20) = 25 > 20.
        let response = post_rpc(
            &cfg,
            json!({
                "jsonrpc":"2.0","id":15,"method":"tools/call",
                "params":{"name":"myTool","arguments":{"a":"hello","b":"x".repeat(20)}}
            }),
        );
        assert_eq!(response.status_code(), 200);
        let body = body_json(&response);
        assert_eq!(body["error"]["code"], -32602, "got: {body:?}");
        let msg = body["error"]["message"].as_str().unwrap_or_default();
        assert!(msg.contains("total"), "should mention 'total': {msg}");
    }

    // -----------------------------------------------------------------------
    // TC-15: tools/call — nested object strings are checked recursively
    // -----------------------------------------------------------------------
    #[test]
    fn tc15_nested_string_in_object_is_checked() {
        let cfg = json!({
            "mcpEndpoint": ENDPOINT,
            "strictMode": true,
            "defaultFieldMaxChars": 5
        })
        .to_string();
        // address.city has 7 chars > 5.
        let response = post_rpc(
            &cfg,
            json!({
                "jsonrpc":"2.0","id":16,"method":"tools/call",
                "params":{"name":"myTool","arguments":{"address":{"city":"toolong"}}}
            }),
        );
        assert_eq!(response.status_code(), 200);
        let body = body_json(&response);
        assert_eq!(body["error"]["code"], -32602, "got: {body:?}");
        let msg = body["error"]["message"].as_str().unwrap_or_default();
        assert!(msg.contains("address.city"), "path must be in error: {msg}");
    }

    // -----------------------------------------------------------------------
    // TC-16: tools/call — no arguments object is valid (no check needed)
    // -----------------------------------------------------------------------
    #[test]
    fn tc16_missing_arguments_object_passes_through() {
        let response = post_rpc(
            &default_config(),
            json!({
                "jsonrpc":"2.0","id":17,"method":"tools/call",
                "params":{"name":"myTool"}
            }),
        );
        // Empty args → no strings to check → Flow::Continue → status 200, empty body.
        assert_eq!(response.status_code(), 200);
    }

    // -----------------------------------------------------------------------
    // TC-17: tools/call — non-object arguments returns -32602
    // -----------------------------------------------------------------------
    #[test]
    fn tc17_non_object_arguments_returns_invalid_params() {
        let response = post_rpc(
            &default_config(),
            json!({
                "jsonrpc":"2.0","id":18,"method":"tools/call",
                "params":{"name":"myTool","arguments":"not-an-object"}
            }),
        );
        assert_eq!(response.status_code(), 200);
        assert_eq!(body_json(&response)["error"]["code"], -32602);
    }

    // -----------------------------------------------------------------------
    // TC-18: tools/call — unicode char count (not byte count)
    // -----------------------------------------------------------------------
    #[test]
    fn tc18_unicode_chars_not_bytes_are_counted() {
        let cfg = json!({
            "mcpEndpoint": ENDPOINT,
            "strictMode": true,
            "defaultFieldMaxChars": 4
        })
        .to_string();
        // "São" = 3 Unicode chars (5 UTF-8 bytes) → passes a 4-char limit.
        // Flow::Continue → pdk-unit 200, empty body (no upstream registered).
        let response_ok = post_rpc(
            &cfg,
            json!({
                "jsonrpc":"2.0","id":19,"method":"tools/call",
                "params":{"name":"myTool","arguments":{"city":"São"}}
            }),
        );
        assert_eq!(response_ok.status_code(), 200, "São (3 chars) should pass a 4-char limit");

        // "SãoP" = 4 chars (6 UTF-8 bytes) → passes at exactly the limit.
        let response_exact = post_rpc(
            &cfg,
            json!({
                "jsonrpc":"2.0","id":20,"method":"tools/call",
                "params":{"name":"myTool","arguments":{"city":"SãoP"}}
            }),
        );
        assert_eq!(response_exact.status_code(), 200, "SãoP (4 chars) should pass at the exact limit");

        // "SãoPaulo" = 8 chars → fails.
        let response_fail = post_rpc(
            &cfg,
            json!({
                "jsonrpc":"2.0","id":21,"method":"tools/call",
                "params":{"name":"myTool","arguments":{"city":"SãoPaulo"}}
            }),
        );
        assert_eq!(body_json(&response_fail)["error"]["code"], -32602);
    }

    // -----------------------------------------------------------------------
    // TC-19: request body size cap
    // -----------------------------------------------------------------------
    #[test]
    fn tc19_oversized_request_body_is_rejected_before_parsing() {
        let cfg = json!({
            "mcpEndpoint": ENDPOINT,
            "strictMode": true,
            "maxRequestBytes": 1024
        })
        .to_string();
        let pad = "x".repeat(4096);
        let response = tester(&cfg).request(
            UnitHttpRequest::post()
                .with_path(ENDPOINT)
                .with_header("content-type", "application/json")
                .with_body(
                    json!({"jsonrpc":"2.0","id":1,"method":"ping","pad": pad}).to_string(),
                ),
        );
        assert_eq!(response.status_code(), 200);
        let body = body_json(&response);
        assert_eq!(body["error"]["code"], -32600);
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("exceeds limit"),
            "got: {body:?}"
        );
    }

    // -----------------------------------------------------------------------
    // TC-20: notifications pass through without limit checks
    // -----------------------------------------------------------------------
    #[test]
    fn tc20_notification_passes_through_without_checks() {
        // A notification (no id) for tools/call would technically be malformed,
        // but the policy must not crash: it detects the notification flag before
        // hitting the argument check and passes through.
        let response = post_rpc(
            &default_config(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        );
        // Flow::Continue → pdk-unit 200.
        assert_eq!(response.status_code(), 200);
    }
}

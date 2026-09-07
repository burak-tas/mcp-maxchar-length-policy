# MCP Tool Argument Character Limit Policy

A MuleSoft Omni Gateway PDK policy that enforces per-field and aggregate character limits on MCP `tools/call` arguments, returning `-32602 Invalid params` before any upstream is reached when a limit is violated.

## Why this policy exists

The MCP specification provides no character-limit enforcement at the protocol level:

| Layer | Mechanism |
|---|---|
| MCP Protocol | None — no cap on argument values |
| Tool's `inputSchema` | `maxLength` is opt-in, per tool, per developer |
| MCP Schema Validation Policy | Only if defined in the tool's own schema |
| **This policy** | Uniform enforcement, no per-tool opt-in required |

A single misconfigured or incomplete tool schema is enough to leave the door open to:
- **Token burn** — oversized strings passed as tool arguments flow into the LLM context
- **Prompt injection** — hidden instructions embedded in long string arguments
- **Memory pressure** — `${args.*}` interpolation of very long values in downstream policies

## How it works

This policy is a **pass-through enforcer** — it sits in front of any MCP backend and adds the character-limit layer without replacing the server:

```
MCP Client
    ↓ POST /mcp
[This Policy]
    ├─ tools/call: check per-field + total char limits
    │   ├─ FAIL → -32602 Invalid params (Flow::Break, no upstream reached)
    │   └─ PASS → Flow::Continue
    └─ All other methods → Flow::Continue (pass through unchanged)
[Your MCP Backend]
```

## Schema Validation vs MCP Tool Argument Character Limit - Test Results

MCP's built-in JSON Schema Validation policy enforces `inputSchema.maxLength` — but only when the tool developer explicitly includes it. The mock tools used here intentionally omit `maxLength` on all fields, as most real-world tools do. The Omni Gateway Schema Validation policy would pass every request below; this policy rejects them.

| DT | Test | Schema Validation result | This policy result |
|---|---|---|---|
| DT-01 | Short query (12 chars) via `searchDocuments` | PASS (no constraint) | PASS |
| DT-02 | Oversized query (2049 chars, default limit 2048) | **PASS — gap exposed** | **REJECT -32602** |
| DT-03 | `description` exactly 512 chars | PASS | PASS |
| DT-04 | `description` 513 chars (field-specific limit 512) | **PASS — gap exposed** | **REJECT -32602** |
| DT-05 | Nested `metadata.author` 2049 chars (recursive check) | **PASS — gap exposed** | **REJECT -32602** |
| DT-06 | Unicode query `"São Paulo"` (9 chars, 11 UTF-8 bytes) | PASS | PASS (chars, not bytes) |


## Configuration reference

| Parameter | Type | Default | Description |
|---|---|---|---|
| `mcpEndpoint` | string | `/mcp` | Full path where the MCP endpoint is served on the Omni Gateway (e.g. `/countrycode/mcp`). Must match the complete URL path — not just the suffix. |
| `strictMode` | boolean | `true` | `true`: non-MCP paths return 404; `false`: fall through |
| `defaultFieldMaxChars` | integer | `4096` | Character limit applied to every string field not covered by `fieldLimits` |
| `fieldLimits` | array | `[]` | Per-field overrides (see below) |
| `maxTotalArgumentChars` | integer | *(unset)* | Maximum combined character count across all string values |
| `maxRequestBytes` | integer | `1048576` | Maximum raw request body size (bytes). Enforced before parsing. |

> **Important — `mcpEndpoint` must be the full Omni Gateway path.**
> The policy matches requests using `starts_with(mcpEndpoint)` against the full `:path` header that Omni Gateway sees. If your API instance is mounted at `/countrycode/mcp`, set `mcpEndpoint: "/countrycode/mcp"` — setting it to just `"/mcp"` will not match and the policy will silently pass all requests through (when `strictMode: false`).

### `fieldLimits` entries

Each entry is an object with:

| Key | Type | Description |
|---|---|---|
| `field` | string | Top-level argument field name, or `"*"` for all unmatched fields |
| `maxChars` | integer | Maximum character count for this field |

**Lookup priority** (per string value):
1. Exact match in `fieldLimits` for the top-level key
2. Wildcard `"*"` entry in `fieldLimits`
3. `defaultFieldMaxChars`
4. No limit

## Example configuration

```yaml
config:
  # Use the full path exposed by Omni Gateway, not just the suffix.
  # Example: if your API is mounted at /countrycode/mcp, set this to /countrycode/mcp.
  mcpEndpoint: "/mcp"
  strictMode: true

  # 2 KB default for any field not explicitly listed
  defaultFieldMaxChars: 2048

  # Tighter limits for prompt-injection-sensitive fields
  fieldLimits:
    - field: "description"
      maxChars: 512
    - field: "prompt"
      maxChars: 512
    - field: "instructions"
      maxChars: 1024

  # Combined cap: all string values together must not exceed 8 KB
  maxTotalArgumentChars: 8192

  # Physical request-body guard (matches the Omni Gateway buffer default)
  maxRequestBytes: 1048576
```

## Behavior on violation

A violation returns a JSON-RPC `-32602 Invalid params` error **before any upstream is reached**:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32602,
    "message": "argument character limit exceeded — argument field 'description' has 600 character(s), which exceeds the limit of 512"
  }
}
```

Error messages include:
- The field path (e.g. `description`, `address.city`)
- The actual character count
- The configured limit

Error messages **never** include the offending string value (which may be prompt-injection material or PII).

## Recursive checking

String values at any depth are checked, not just top-level fields. Given:

```json
{"address": {"city": "a-very-long-city-name"}}
```

The path in the error is `address.city`, and the limit applied is the one for the `address` top-level key (or `defaultFieldMaxChars` if `address` has no entry in `fieldLimits`).

## Combining with the MCP Tool Composer Policy

This policy is designed to be chained **before** the MCP Tool Composer Policy in the Omni Gateway policy chain:

```
[MCP Tool Argument Character Limit Policy]  ← enforces limits
          ↓
[MCP Tool Composer Policy]                  ← executes the pipeline
```

## Policy ordering

```yaml
policies:
  - policyRef:
      name: mcp-maxchar-length-policy-v0-1-impl
      namespace: default
    config: { ... }
  - policyRef:
      name: mcp-tool-composer-policy-v0-1-impl
      namespace: default
    config: { ... }
```

## Local development

### Prerequisites

- Rust ≥ 1.88.0 with `wasm32-wasip1` target
- `cargo-anypoint` 1.10.0 (`cargo install cargo-anypoint@1.10.0`)
- `anypoint-cli-v4` with PDK plugin
- Docker (for `make run`)

### Run the test suite

```bash
make test
# or directly:
cargo test --lib
```

### Build the WASM artifact

```bash
make build
```

### Run locally in Docker (Omni Gateway 1.12+)

```bash
make run
```

Then test against `http://localhost:8081/mcp`.

## Test coverage

42 automated tests across three modules:

| Module | Tests | Coverage |
|---|---|---|
| `argcheck` | 14 | Per-field limits (default, specific, wildcard), total limit, recursion (object, array), Unicode chars, error sanitization |
| `config` | 7 | Defaults, field-limit parsing, zero-value rejection, endpoint normalization |
| Integration (`tests`) | 21 | Transport guards, envelope validation, pass-through, limit enforcement, unicode, request-size cap, notifications |

## Versioning and minimum runtime

- **Policy version**: `0.1.0`
- **Minimum Omni Gateway**: `1.12.0` (requires `flex_enable_stop_iteration` ABI for atomic header+body buffering)
- **PDK version**: `1.10.0`

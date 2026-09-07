# TEST_REPORT — MCP Tool Argument Character Limit Policy

**Commit:** *(initial)*
**Date:** 2026-09-07
**Test runner:** `cargo test --lib` via pdk-unit
**Result:** **42 / 42 PASS**

---

## Summary

| Suite | Tests | Pass | Fail |
|---|---|---|---|
| `argcheck` (unit) | 14 | 14 | 0 |
| `config` (unit) | 7 | 7 | 0 |
| Integration (`tests`) | 21 | 21 | 0 |
| **Total** | **42** | **42** | **0** |

---

## argcheck unit tests

| # | Test | Description | Result |
|---|---|---|---|
| 1 | `short_string_passes_default_limit` | String under default limit passes | PASS |
| 2 | `string_at_exact_limit_passes` | String at the exact limit passes | PASS |
| 3 | `string_one_over_default_limit_fails` | One char over limit returns error naming field + counts | PASS |
| 4 | `no_limits_configured_allows_any_string` | No limits at all → any string passes | PASS |
| 5 | `specific_field_limit_overrides_default` | Field-specific limit overrides defaultFieldMaxChars | PASS |
| 6 | `other_field_falls_back_to_default_when_specific_limit_set` | Fields not in fieldLimits use default | PASS |
| 7 | `wildcard_applies_when_no_exact_match` | `"*"` entry applies to unmatched fields | PASS |
| 8 | `exact_match_takes_priority_over_wildcard` | Exact match beats wildcard for same field | PASS |
| 9 | `nested_string_inside_object_is_checked` | String inside nested object is checked; path `a.b` reported | PASS |
| 10 | `string_inside_array_is_checked` | String inside array item is checked; path `tags[1]` reported | PASS |
| 11 | `total_limit_sums_all_strings` | Total of all string chars is summed and checked | PASS |
| 12 | `total_check_runs_after_per_field_passes` | Per-field can pass while total fails | PASS |
| 13 | `unicode_chars_counted_not_bytes` | `chars().count()` used, not `len()` (UTF-8 bytes) | PASS |
| 14 | `error_never_echoes_the_offending_string_value` | Error message contains field name but NOT the string value | PASS |

---

## config unit tests

| # | Test | Description | Result |
|---|---|---|---|
| 1 | `defaults_apply_when_nothing_configured` | Default 4096, strict_mode=true, 1 MiB limit | PASS |
| 2 | `field_limits_parsed_correctly` | Named field + wildcard entries parsed as (name, usize) tuples | PASS |
| 3 | `zero_default_max_is_rejected` | defaultFieldMaxChars=0 returns InvalidDefaultMax error | PASS |
| 4 | `zero_field_max_is_rejected` | fieldLimits[0].maxChars=0 returns InvalidFieldMax error | PASS |
| 5 | `empty_field_name_is_rejected` | Blank field name returns EmptyFieldName error | PASS |
| 6 | `total_max_zero_is_rejected` | maxTotalArgumentChars=0 returns InvalidTotalMax error | PASS |
| 7 | `mcp_endpoint_gets_leading_slash` | Endpoint `"mcp"` normalised to `"/mcp"` | PASS |

---

## Integration tests (pdk-unit, driving `configure` entrypoint)

| TC | Test | What is asserted | Result |
|---|---|---|---|
| TC-01 | `tc01_non_mcp_path_is_404_in_strict_mode` | Non-MCP path + strictMode=true → 404 | PASS |
| TC-02 | `tc02_non_mcp_path_passes_through_in_non_strict_mode` | Non-MCP path + strictMode=false → Flow::Continue | PASS |
| TC-03 | `tc03_get_passes_through` | GET → Flow::Continue (SSE handshake for mcp-support-policy) | PASS |
| TC-04 | `tc04_wrong_content_type_returns_invalid_request` | Content-Type: text/plain → -32600 | PASS |
| TC-05 | `tc05_malformed_json_returns_parse_error` | Invalid JSON body → -32700 | PASS |
| TC-06 | `tc06_wrong_jsonrpc_version_returns_invalid_request` | jsonrpc: "1.0" → -32600 | PASS |
| TC-07 | `tc07_missing_method_returns_invalid_request` | Missing method field → -32600 | PASS |
| TC-08 | `tc08_unknown_protocol_version_passes_through` | Any MCP-Protocol-Version passes through — version negotiation is mcp-support-policy's job | PASS |
| TC-09 | `tc09_initialize_passes_through_downstream` | `initialize` with no char limits → Flow::Continue | PASS |
| TC-09b | `tc09b_tools_list_passes_through_downstream` | `tools/list` → Flow::Continue | PASS |
| TC-10 | `tc10_tools_call_within_default_limit_passes_through` | tools/call with short arg → Flow::Continue | PASS |
| TC-11 | `tc11_single_field_over_default_limit_is_invalid_params` | 11-char value, 10-char limit → -32602; error names field, never echoes value | PASS |
| TC-12 | `tc12_field_specific_limit_overrides_default` | description: 22 chars over field-specific 20-char limit → -32602; short value passes | PASS |
| TC-13 | `tc13_wildcard_field_limit_applies_to_unmatched_fields` | Wildcard `"*"` limit of 5 catches `anyfield: "sixchr"` → -32602 | PASS |
| TC-14 | `tc14_total_char_limit_exceeded_is_invalid_params` | Per-field fine, but total 25 > 20 cap → -32602 with "total" in message | PASS |
| TC-15 | `tc15_nested_string_in_object_is_checked` | `address.city` = 7 chars, default 5-char limit → -32602; path `address.city` in error | PASS |
| TC-16 | `tc16_missing_arguments_object_passes_through` | No `arguments` key → Flow::Continue | PASS |
| TC-17 | `tc17_non_object_arguments_returns_invalid_params` | `arguments: "not-an-object"` → -32602 | PASS |
| TC-18 | `tc18_unicode_chars_not_bytes_are_counted` | "São" (3 chars, 5 bytes) passes 4-char limit; "SãoPaulo" (8 chars) fails | PASS |
| TC-19 | `tc19_oversized_request_body_is_rejected_before_parsing` | Body > maxRequestBytes (1024) → -32600 before parse | PASS |
| TC-20 | `tc20_notification_passes_through_without_checks` | Id-less notification → Flow::Continue, no limit check | PASS |

---

## Issues deliberately avoided from mcp-tool-composer-policy history

The following classes of bugs found in the composer policy were designed out from the start:

| Composer Issue | How avoided in this policy |
|---|---|
| #1 DataWeave payload binding panic | This policy has no DataWeave — no scripting engine used |
| #2 Missing .project.yaml | `.project.yaml` present from day 0 |
| #3/#14 Transport non-compliance (protocol version, GET SSE) | Full Streamable-HTTP transport guards implemented from day 0 |
| #4/#12 Arguments not validated against schema | This policy *is* the validation layer — core purpose |
| #5/#11 Raw string interpolation injection | This policy does not interpolate — it only reads and measures |
| #6/#15 enable_stop_iteration missing | Both `pdk` and `pdk-unit` carry the feature flag from day 0 |
| #7/#13 Credential leak in errors | No credentials handled; error messages never echo string values |
| #8 Tool execution failures as INTERNAL_ERROR | CallToolResult semantics correct from day 0 (pass-through, not terminating) |
| #9/#16 No payload-size limits | `maxRequestBytes` enforced before parsing from day 0 |
| #17/#18 No CI | GitHub Actions CI added from day 0 |

---

## Live deployment tests — Schema Validation gap demonstration

**Environment:** Flex Gateway 1.12.0 (Docker, local playground)  
**Backend:** Node.js mock MCP server (serves `searchDocuments` and `createRecord`)  
**Policy config:** `defaultFieldMaxChars: 2048`, `fieldLimits: [{field: "description", maxChars: 512}]`  
**Date:** 2026-09-07

### Why Schema Validation does not close this gap

MCP's built-in JSON Schema Validation policy enforces `inputSchema.maxLength` — but only when the tool developer explicitly includes it. The mock tools used here intentionally omit `maxLength` on all fields, as most real-world tools do. The Flex Gateway Schema Validation policy would pass every request below; this policy rejects them.

| DT | Test | Schema Validation result | This policy result |
|---|---|---|---|
| DT-01 | Short query (12 chars) via `searchDocuments` | PASS (no constraint) | PASS |
| DT-02 | Oversized query (2049 chars, default limit 2048) | **PASS — gap exposed** | **REJECT -32602** |
| DT-03 | `description` exactly 512 chars | PASS | PASS |
| DT-04 | `description` 513 chars (field-specific limit 512) | **PASS — gap exposed** | **REJECT -32602** |
| DT-05 | Nested `metadata.author` 2049 chars (recursive check) | **PASS — gap exposed** | **REJECT -32602** |
| DT-06 | Unicode query `"São Paulo"` (9 chars, 11 UTF-8 bytes) | PASS | PASS (chars, not bytes) |

### DT-02 — the primary gap case

Request:
```json
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"searchDocuments","arguments":{"query":"AAAA…(2049 chars)"}}}
```

Schema Validation: passes — `inputSchema` declares `query: {type: string}` with no `maxLength`.

This policy response:
```json
{"jsonrpc":"2.0","id":2,"error":{"code":-32602,"message":"argument character limit exceeded — argument field 'query' has 2049 character(s), which exceeds the limit of 2048"}}
```

### DT-04 — field-specific limit gap

Request:
```json
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"createRecord","arguments":{"title":"My Title","description":"CCCC…(513 chars)"}}}
```

Schema Validation: passes — `description: {type: string}` has no `maxLength`.

This policy response:
```json
{"jsonrpc":"2.0","id":4,"error":{"code":-32602,"message":"argument character limit exceeded — argument field 'description' has 513 character(s), which exceeds the limit of 512"}}
```

### DT-05 — recursive nested-object gap

Request:
```json
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"createRecord","arguments":{"title":"Test","metadata":{"author":"DDDD…(2049 chars)"}}}}
```

Schema Validation: passes — `metadata: {type: object}` has no property-level `maxLength` constraints.

This policy response:
```json
{"jsonrpc":"2.0","id":5,"error":{"code":-32602,"message":"argument character limit exceeded — argument field 'metadata.author' has 2049 character(s), which exceeds the limit of 2048"}}
```

Error path `metadata.author` confirms the recursive tree-walk fired at depth 2.

### Summary

| Scenario | Schema Validation alone | This policy |
|---|---|---|
| Tool has `maxLength` in schema | Enforces it | Enforces the lower of the two limits |
| Tool has **no** `maxLength` | **No enforcement — any string passes** | Enforces `fieldLimits` / `defaultFieldMaxChars` |
| Nested object strings | Never checked (Schema Validation is top-level only) | Recursively enforced at any depth |
| Total aggregate cap across all fields | Not supported | `maxTotalArgumentChars` enforces it |

This policy provides **uniform, unconditional enforcement** regardless of whether individual tool schemas declare `maxLength` — closing the gap that Schema Validation leaves open for every tool that omits it.

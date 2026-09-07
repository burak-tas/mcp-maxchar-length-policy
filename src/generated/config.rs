use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct FieldLimits0Config {
    #[serde(alias = "field")]
    pub field: String,
    #[serde(alias = "maxChars")]
    pub max_chars: i64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    #[serde(alias = "defaultFieldMaxChars")]
    pub default_field_max_chars: Option<i64>,
    #[serde(alias = "fieldLimits")]
    pub field_limits: Option<Vec<FieldLimits0Config>>,
    #[serde(alias = "maxRequestBytes")]
    pub max_request_bytes: Option<i64>,
    #[serde(alias = "maxTotalArgumentChars")]
    pub max_total_argument_chars: Option<i64>,
    #[serde(alias = "mcpEndpoint")]
    pub mcp_endpoint: Option<String>,
    #[serde(alias = "strictMode")]
    pub strict_mode: Option<bool>,
}

#[pdk::hl::entrypoint_flex]
fn init(abi: &dyn pdk::flex_abi::api::FlexAbi) -> Result<(), anyhow::Error> {
    let _config: Config = serde_json::from_slice(abi.get_configuration())
        .map_err(|err| {
            anyhow::anyhow!(
                "Failed to parse configuration '{}'. Cause: {}",
                String::from_utf8_lossy(abi.get_configuration()),
                err
            )
        })?;
    abi.setup()?;
    Ok(())
}

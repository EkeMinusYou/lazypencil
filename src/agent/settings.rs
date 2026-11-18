use super::{agent_error, protocol_error};
use crate::{Error, Result};
use agent_client_protocol::{
    Agent, ConnectionTo,
    schema::v1::{
        SessionConfigValueId, SessionId, SetSessionConfigOptionRequest, SetSessionModeRequest,
    },
};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionSelector {
    Model,
    Speed,
}

impl SessionSelector {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Speed => "speed",
        }
    }

    pub(crate) fn option(self, options: &Value) -> Option<&Value> {
        unique_option(options, |option| match self {
            Self::Model => option["category"] == "model" || option["id"] == "model",
            Self::Speed => option["id"] == "fast-mode",
        })
    }

    pub(crate) fn choices(self, options: &Value) -> impl Iterator<Item = &Value> {
        self.option(options)
            .filter(|option| option["type"] == "select")
            .and_then(|option| option["options"].as_array())
            .into_iter()
            .flatten()
            .flat_map(|option| {
                option["options"]
                    .as_array()
                    .map_or(std::slice::from_ref(option), Vec::as_slice)
            })
            .filter(|option| option["value"].is_string())
    }
}

pub(crate) fn unique_option(
    options: &Value,
    mut matches: impl FnMut(&Value) -> bool,
) -> Option<&Value> {
    let mut matching = options.as_array()?.iter().filter(|option| matches(option));
    let option = matching.next()?;
    matching.next().is_none().then_some(option)
}

fn mode_selector(response: &Value) -> Option<&Value> {
    let options = response["configOptions"].as_array()?;
    let mut modes = options.iter().filter(|item| {
        (item["id"] == "mode" || item["category"] == "mode") && item["type"] == "select"
    });
    let mode = modes.next()?;
    if modes.next().is_some() {
        None
    } else {
        Some(mode)
    }
}

pub(crate) fn normalize_modes(response: &mut Value) {
    let Some(selector) = mode_selector(response).cloned() else {
        return;
    };
    fn collect(options: &Value, output: &mut Vec<Value>) {
        if let Some(options) = options.as_array() {
            for option in options.iter().take(128) {
                if output.len() >= 128 {
                    break;
                }
                if option["value"].is_string() {
                    output.push(json!({"id":option["value"],"name":option["name"],"description":option["description"]}));
                } else {
                    collect(&option["options"], output);
                }
            }
        }
    }
    let mut modes = Vec::new();
    collect(&selector["options"], &mut modes);
    response["modes"] = json!({"currentModeId":selector["currentValue"],"availableModes":modes});
}

pub(super) fn merge_updates(
    response: &mut Value,
    session: &SessionId,
    updates: &std::sync::Mutex<Value>,
) {
    let value = updates.lock().expect("mode updates").take();
    if value["sessionId"] != session.0.as_ref() {
        return;
    }
    if let Some(options) = value.get("configOptions") {
        response["configOptions"] = options.clone();
        normalize_modes(response);
    }
    if let Some(mode) = value.get("currentModeId") {
        response["modes"]["currentModeId"] = mode.clone();
    }
}

async fn set_config_option(
    connection: &ConnectionTo<Agent>,
    session: &SessionId,
    response: &mut Value,
    id: &str,
    value: &str,
    setting: &str,
    diagnostic_directory: Option<&std::path::Path>,
) -> Result<()> {
    let changed = connection
        .send_request(SetSessionConfigOptionRequest::new(
            session.clone(),
            id.to_owned(),
            SessionConfigValueId::new(value.to_owned()),
        ))
        .block_task()
        .await
        .map_err(|error| {
            protocol_error("session/set_config_option", error, diagnostic_directory)
        })?;
    response["configOptions"] = serde_json::to_value(changed.config_options)
        .map_err(|error| agent_error(error.to_string()))?;
    normalize_modes(response);
    if !unique_option(&response["configOptions"], |option| option["id"] == id)
        .is_some_and(|option| option["currentValue"] == value)
    {
        return Err(agent_error(format!(
            "The adapter did not confirm the selected {setting}"
        )));
    }
    Ok(())
}

pub(super) async fn select(
    connection: &ConnectionTo<Agent>,
    session: &SessionId,
    response: &mut Value,
    selector: SessionSelector,
    value: &str,
    diagnostic_directory: Option<&std::path::Path>,
) -> Result<()> {
    let options = &response["configOptions"];
    let name = selector.name();
    let id = selector
        .option(options)
        .and_then(|option| option["id"].as_str())
        .filter(|id| unique_option(options, |option| option["id"] == *id).is_some())
        .filter(|_| {
            selector
                .choices(options)
                .any(|choice| choice["value"] == value)
        })
        .ok_or_else(|| {
            Error::new(
                "INVALID_ARGUMENT",
                format!("The requested {name} is not advertised by the adapter"),
            )
        })?
        .to_owned();
    set_config_option(
        connection,
        session,
        response,
        &id,
        value,
        name,
        diagnostic_directory,
    )
    .await
}

pub(super) async fn apply(
    connection: &ConnectionTo<Agent>,
    session: &SessionId,
    response: &mut Value,
    mode: Option<&str>,
    model: Option<&str>,
    fast: Option<bool>,
    diagnostic_directory: Option<&std::path::Path>,
) -> Result<()> {
    if let Some(model) = model {
        select(
            connection,
            session,
            response,
            SessionSelector::Model,
            model,
            diagnostic_directory,
        )
        .await?;
    }
    // Model changes can replace the available speed controls, so select speed afterward.
    if let Some(fast) = fast {
        select(
            connection,
            session,
            response,
            SessionSelector::Speed,
            if fast { "on" } else { "off" },
            diagnostic_directory,
        )
        .await?;
    }
    normalize_modes(response);
    if let Some(mode) = mode {
        let available = response["modes"]["availableModes"].as_array();
        if !available.is_some_and(|modes| modes.iter().any(|candidate| candidate["id"] == mode)) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "The requested mode is not advertised by the adapter",
            )
            .details(json!({"requested_mode":mode,"available_modes":available})));
        }
        let selector_id = mode_selector(response)
            .and_then(|item| item["id"].as_str())
            .map(str::to_owned);
        if let Some(selector_id) = selector_id {
            set_config_option(
                connection,
                session,
                response,
                &selector_id,
                mode,
                "permission mode",
                diagnostic_directory,
            )
            .await?;
        } else {
            // ACP set_mode's successful response acknowledges the selected mode.
            connection
                .send_request(SetSessionModeRequest::new(session.clone(), mode.to_owned()))
                .block_task()
                .await
                .map_err(|error| protocol_error("session/set_mode", error, diagnostic_directory))?;
        }
        response["modes"]["currentModeId"] = json!(mode);
    }
    Ok(())
}

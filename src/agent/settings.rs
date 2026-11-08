use super::{agent_error, protocol_error};
use crate::{Error, Result};
use agent_client_protocol::{
    Agent, ConnectionTo,
    schema::v1::{
        SessionConfigValueId, SessionId, SetSessionConfigOptionRequest, SetSessionModeRequest,
    },
};
use serde_json::{Value, json};

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

pub(super) async fn apply(
    connection: &ConnectionTo<Agent>,
    session: &SessionId,
    response: &mut Value,
    mode: Option<&str>,
    model: Option<&str>,
    fast: Option<bool>,
) -> Result<()> {
    if let Some(model) = model {
        let configs = response["configOptions"].as_array().ok_or_else(|| {
            Error::new(
                "INVALID_ARGUMENT",
                "This adapter does not advertise model selection",
            )
        })?;
        let models: Vec<_> = configs
            .iter()
            .filter(|option| option["category"] == "model" || option["id"] == "model")
            .collect();
        if models.len() != 1 || models[0]["type"] != "select" {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "The adapter did not advertise one unambiguous model selector",
            ));
        }
        let option = models[0];
        let id = option["id"]
            .as_str()
            .ok_or_else(|| agent_error("The model selector has no ID"))?
            .to_owned();
        if !contains_value(&option["options"], model) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "The requested model is not advertised by the adapter",
            )
            .details(json!({"requested_model":model,"model_options":option["options"]})));
        }
        let changed = connection
            .send_request(SetSessionConfigOptionRequest::new(
                session.clone(),
                id.clone(),
                SessionConfigValueId::new(model.to_owned()),
            ))
            .block_task()
            .await
            .map_err(|error| protocol_error("session/set_config_option", error))?;
        let configs = serde_json::to_value(changed.config_options)
            .map_err(|error| agent_error(error.to_string()))?;
        if !configs.as_array().is_some_and(|options| {
            options
                .iter()
                .any(|option| option["id"] == id && option["currentValue"] == model)
        }) {
            return Err(agent_error(
                "The adapter did not confirm the selected model",
            ));
        }
        response["configOptions"] = configs;
    }
    // Model changes can replace the available speed controls, so select speed afterward.
    if let Some(fast) = fast {
        let value = if fast { "on" } else { "off" };
        let options: Vec<_> = response["configOptions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|option| option["id"] == "fast-mode")
            .collect();
        if options.len() != 1
            || options[0]["type"] != "select"
            || !contains_value(&options[0]["options"], value)
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "The adapter does not advertise the requested Fast mode setting for this model",
            ));
        }
        let changed = connection
            .send_request(SetSessionConfigOptionRequest::new(
                session.clone(),
                "fast-mode".to_owned(),
                SessionConfigValueId::new(value.to_owned()),
            ))
            .block_task()
            .await
            .map_err(|error| protocol_error("session/set_config_option", error))?;
        let configs = serde_json::to_value(changed.config_options)
            .map_err(|error| agent_error(error.to_string()))?;
        let confirmed: Vec<_> = configs
            .as_array()
            .into_iter()
            .flatten()
            .filter(|option| option["id"] == "fast-mode")
            .collect();
        if confirmed.len() != 1 || confirmed[0]["currentValue"] != value {
            return Err(agent_error(
                "The adapter did not confirm the selected Fast mode",
            ));
        }
        response["configOptions"] = configs;
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
            let changed = connection
                .send_request(SetSessionConfigOptionRequest::new(
                    session.clone(),
                    selector_id.clone(),
                    SessionConfigValueId::new(mode.to_owned()),
                ))
                .block_task()
                .await
                .map_err(|error| protocol_error("session/set_config_option", error))?;
            let configs = serde_json::to_value(changed.config_options)
                .map_err(|error| agent_error(error.to_string()))?;
            response["configOptions"] = configs.clone();
            response["modes"]["currentModeId"] = configs
                .as_array()
                .and_then(|items| items.iter().find(|item| item["id"] == selector_id))
                .map(|item| item["currentValue"].clone())
                .unwrap_or(Value::Null);
            if !configs.as_array().is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item["id"] == selector_id && item["currentValue"] == mode)
            }) {
                return Err(agent_error(
                    "The adapter did not confirm the selected permission mode",
                ));
            }
            response["configOptions"] = configs;
        } else {
            // ACP set_mode's successful response acknowledges the selected mode.
            connection
                .send_request(SetSessionModeRequest::new(session.clone(), mode.to_owned()))
                .block_task()
                .await
                .map_err(|error| protocol_error("session/set_mode", error))?;
        }
        response["modes"]["currentModeId"] = json!(mode);
    }
    Ok(())
}

fn contains_value(options: &Value, value: &str) -> bool {
    options.as_array().is_some_and(|items| {
        items
            .iter()
            .any(|option| option["value"] == value || contains_value(&option["options"], value))
    })
}

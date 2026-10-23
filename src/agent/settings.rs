use super::{agent_error, protocol_error};
use crate::{Error, Result};
use agent_client_protocol::{
    Agent, ConnectionTo,
    schema::v1::{
        SessionConfigValueId, SessionId, SetSessionConfigOptionRequest, SetSessionModeRequest,
    },
};
use serde_json::{Value, json};

pub(super) async fn apply(
    connection: &ConnectionTo<Agent>,
    session: &SessionId,
    response: &mut Value,
    mode: Option<&str>,
    model: Option<&str>,
) -> Result<()> {
    if let Some(mode) = mode {
        let available = response["modes"]["availableModes"].as_array();
        if !available.is_some_and(|modes| modes.iter().any(|candidate| candidate["id"] == mode)) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "The requested mode is not advertised by the adapter",
            )
            .details(json!({"requested_mode":mode,"available_modes":available})));
        }
        connection
            .send_request(SetSessionModeRequest::new(session.clone(), mode.to_owned()))
            .block_task()
            .await
            .map_err(protocol_error)?;
        response["modes"]["currentModeId"] = json!(mode);
    }
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
            .map_err(protocol_error)?;
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
    Ok(())
}

fn contains_value(options: &Value, value: &str) -> bool {
    options.as_array().is_some_and(|items| {
        items
            .iter()
            .any(|option| option["value"] == value || contains_value(&option["options"], value))
    })
}

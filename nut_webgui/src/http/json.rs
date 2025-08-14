use super::{RouterState, problem_detail::ProblemDetail};
use crate::{
  config::UpsdConfig,
  device_entry::{DeviceEntry, VarDetail},
};
use axum::{
  Json,
  extract::{
    Path, Query, State,
    rejection::{JsonRejection, PathRejection},
  },
  http::StatusCode,
  response::{IntoResponse, Response},
};
use nut_webgui_upsmc::InstCmd;
use nut_webgui_upsmc::errors::{ErrorKind, ProtocolError};
use nut_webgui_upsmc::{CmdName, UpsName, Value, VarName, clients::NutAuthClient};
use once_cell::sync::Lazy;
use std::{collections::HashMap, time::{Duration, Instant}};
use tokio::sync::Mutex;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

macro_rules! require_auth_config {
  ($config:expr) => {
    match $config {
      upsd @ UpsdConfig {
        pass: Some(pass),
        user: Some(user),
        ..
      } => Ok((upsd.get_socket_addr(), user.as_ref(), pass.as_ref())),
      _ => Err(
        ProblemDetail::new("Insufficient upsd configuration", StatusCode::UNAUTHORIZED)
          .with_detail("Operation requires valid username and password to be configured.".into()),
      ),
    }
  };
}

static COMMAND_CACHE: Lazy<Mutex<HashMap<UpsName, (Instant, Vec<InstCmd>)>>> = Lazy::new(|| Mutex::new(HashMap::new()));

async fn get_commands_cached(
  rs: &RouterState,
  ups_name: &UpsName,
  force: bool,
) -> Vec<InstCmd> {
  let now = Instant::now();
  if !force {
    let cache = COMMAND_CACHE.lock().await;
    if let Some((exp, cmds)) = cache.get(ups_name) {
      if *exp > now {
        return cmds.clone();
      }
    }
  }
  let (addr, user, password) = match (&rs.config.upsd.user, &rs.config.upsd.pass) {
    (Some(user), Some(pass)) => (rs.config.upsd.get_socket_addr(), user.as_ref(), pass.as_ref()),
    _ => return Vec::new(),
  };
  let mut client = match NutAuthClient::connect(addr, user, password).await {
    Ok(c) => c,
    Err(err) => {
      warn!(message = "failed to list instcmds", device = %ups_name, reason = %err);
      return Vec::new();
    }
  };
  let cmds = match client.list_instcmds(ups_name).await {
    Ok(v) => v,
    Err(err) => {
      warn!(message = "failed to list instcmds", device = %ups_name, reason = %err);
      Vec::new()
    }
  };
  _ = client.close().await;
  let ttl = if cmds.is_empty() { Duration::from_secs(1) } else { Duration::from_secs(2) };
  let exp = now + ttl;
  let mut cache = COMMAND_CACHE.lock().await;
  cache.insert(ups_name.clone(), (exp, cmds.clone()));
  cmds
}

#[derive(Debug, Deserialize)]
pub struct CommandRequest {
  instcmd: CmdName,
}

#[derive(Debug, Deserialize)]
pub struct RwRequest {
  variable: VarName,
  value: Value,
}

pub async fn get_ups_by_name(
  State(rs): State<RouterState>,
  ups_name: Result<Path<UpsName>, PathRejection>,
  Query(query): Query<GetUpsQuery>,
) -> Result<Response, ProblemDetail> {
  let Path(ups_name) = ups_name?;
  let force = query.include.as_deref() == Some("commands");
  let server_state = rs.state.read().await;
  if let Some(ups) = server_state.devices.get(&ups_name) {
    let (power_w, approx) = {
      let mut approx = false;
      let value = if let Some(v) = ups
        .variables
        .get(VarName::UPS_REALPOWER)
        .and_then(|v| v.as_lossly_f64())
      {
        Some(v)
      } else if let Some(v) = ups
        .variables
        .get(VarName::UPS_POWER)
        .and_then(|v| v.as_lossly_f64())
      {
        Some(v)
      } else {
        let load = ups
          .variables
          .get(VarName::UPS_LOAD)
          .and_then(|v| v.as_lossly_f64());
        let nominal = ups
          .variables
          .get(VarName::UPS_REALPOWER_NOMINAL)
          .and_then(|v| v.as_lossly_f64())
          .or_else(|| {
            ups
              .variables
              .get(VarName::UPS_POWER_NOMINAL)
              .and_then(|v| v.as_lossly_f64())
          });
        match (load, nominal) {
          (Some(load), Some(nominal)) => {
            approx = true;
            Some((nominal * load / 100.0).round())
          }
          _ => None,
        }
      };
      (value, approx)
    };
    let mut value = serde_json::to_value(ups).unwrap();
    drop(server_state);
    let cmds = get_commands_cached(&rs, &ups_name, force).await;
    value["commands"] = serde_json::to_value(cmds).unwrap();
    value["power_is_approx"] = approx.into();
    if let Some(p) = power_w {
      if let Some(n) = serde_json::Number::from_f64(p) {
        value["power_w"] = serde_json::Value::Number(n);
      } else {
        value["power_w"] = serde_json::Value::Null;
      }
    } else {
      value["power_w"] = serde_json::Value::Null;
    }
    Ok(Json(value).into_response())
  } else {
    Err(ProblemDetail::new(
      "Device not found",
      StatusCode::NOT_FOUND,
    ))
  }
}

pub async fn get_ups_list(State(rs): State<RouterState>) -> Response {
  let server_state = rs.state.read().await;
  let mut device_refs: Vec<&DeviceEntry> = server_state.devices.values().collect();
  device_refs.sort_by(|r, l| r.name.cmp(&l.name));

  Json(device_refs).into_response()
}

#[derive(Default, Deserialize)]
pub struct GetUpsQuery {
  include: Option<String>,
}

pub async fn post_command(
  State(rs): State<RouterState>,
  ups_name: Result<Path<UpsName>, PathRejection>,
  body: Result<Json<CommandRequest>, JsonRejection>,
) -> Result<StatusCode, ProblemDetail> {
  let Path(ups_name) = ups_name?;
  let Json(body) = body?;
  let (addr, user, password) = require_auth_config!(&rs.config.upsd)?;

  {
    let server_state = rs.state.read().await;

    match server_state.devices.get(&ups_name) {
      Some(device) => {
        if device.commands.contains(&body.instcmd) {
          Ok(())
        } else {
          Err(
            ProblemDetail::new("Invalid INSTCMD", StatusCode::BAD_REQUEST).with_detail(format!(
              "'{cmd_name}' is not listed as supported command on device details.",
              cmd_name = &body.instcmd
            )),
          )
        }
      }
      None => Err(ProblemDetail::new(
        "Device not found",
        StatusCode::NOT_FOUND,
      )),
    }
  }?;

  let mut client = NutAuthClient::connect(addr, user, password).await?;

  {
    let response = client.instcmd(&ups_name, &body.instcmd).await;
    _ = client.close().await;

    response
  }?;

  info!(
    message = "instcmd called",
    device = %ups_name,
    instcmd = %&body.instcmd
  );

  Ok(StatusCode::ACCEPTED)
}

#[derive(Serialize)]
struct InstCmdResponse<'a> {
  ups: &'a UpsName,
  as_user: &'a str,
  count: usize,
  commands: Vec<InstCmd>,
}

pub async fn get_instcmds(
  State(rs): State<RouterState>,
  ups_name: Result<Path<UpsName>, PathRejection>,
) -> Result<Response, ProblemDetail> {
  if !rs.config.allow_instcmds_list {
    return Err(ProblemDetail::new(
      "Target resource not found",
      StatusCode::NOT_FOUND,
    ));
  }
  let Path(ups_name) = ups_name?;
  let (addr, user, password) = require_auth_config!(&rs.config.upsd)?;
  let mut client = match NutAuthClient::connect(addr, user, password).await {
    Ok(c) => c,
    Err(err) => {
      return match err.kind() {
        ErrorKind::ProtocolError {
          inner: ProtocolError::AccessDenied,
        } => Err(ProblemDetail::new(
          "Access denied",
          StatusCode::UNAUTHORIZED,
        )),
        ErrorKind::ProtocolError {
          inner: ProtocolError::UnknownUps,
        } => Err(ProblemDetail::new(
          "Device not found",
          StatusCode::NOT_FOUND,
        )),
        ErrorKind::IOError { .. } | ErrorKind::RequestTimeout => Err(ProblemDetail::new(
          "UPS daemon unreachable",
          StatusCode::BAD_GATEWAY,
        )),
        _ => Err(err.into()),
      };
    }
  };
  let cmds = match client.list_instcmds(&ups_name).await {
    Ok(c) => c,
    Err(err) => {
      return match err.kind() {
        ErrorKind::ProtocolError {
          inner: ProtocolError::AccessDenied,
        } => Err(ProblemDetail::new(
          "Access denied",
          StatusCode::UNAUTHORIZED,
        )),
        ErrorKind::ProtocolError {
          inner: ProtocolError::UnknownUps,
        } => Err(ProblemDetail::new(
          "Device not found",
          StatusCode::NOT_FOUND,
        )),
        ErrorKind::IOError { .. } | ErrorKind::RequestTimeout => Err(ProblemDetail::new(
          "UPS daemon unreachable",
          StatusCode::BAD_GATEWAY,
        )),
        _ => Err(err.into()),
      };
    }
  };
  _ = client.close().await;
  let response = InstCmdResponse {
    ups: &ups_name,
    as_user: user,
    count: cmds.len(),
    commands: cmds,
  };
  Ok(Json(response).into_response())
}

pub async fn post_fsd(
  State(rs): State<RouterState>,
  ups_name: Result<Path<UpsName>, PathRejection>,
) -> Result<StatusCode, ProblemDetail> {
  let Path(ups_name) = ups_name?;
  let (addr, user, password) = require_auth_config!(&rs.config.upsd)?;

  {
    let server_state = rs.state.read().await;
    if server_state.devices.contains_key(&ups_name) {
      Ok(())
    } else {
      Err(ProblemDetail::new(
        "Device not found",
        StatusCode::NOT_FOUND,
      ))
    }
  }?;

  let mut client = NutAuthClient::connect(addr, user, password).await?;

  {
    let response = client.fsd(&ups_name).await;
    _ = client.close().await;

    response
  }?;

  warn!(
    message = "force shutdown (fsd) called",
    device = %ups_name,
  );

  Ok(StatusCode::ACCEPTED)
}

pub async fn patch_var(
  State(rs): State<RouterState>,
  ups_name: Result<Path<UpsName>, PathRejection>,
  body: Result<Json<RwRequest>, JsonRejection>,
) -> Result<StatusCode, ProblemDetail> {
  let Path(ups_name) = ups_name?;
  let Json(body) = body?;
  let (addr, user, password) = require_auth_config!(&rs.config.upsd)?;

  {
    let server_state = rs.state.read().await;

    match server_state.devices.get(&ups_name) {
      Some(device) => match device.rw_variables.get(&body.variable) {
        Some(VarDetail::Number) => {
          if body.value.is_numeric() {
            Ok(())
          } else {
            Err(
              ProblemDetail::new("Invalid value type", StatusCode::BAD_REQUEST).with_detail(
                format!(
                  "'{var_name}' expects a numeric type, but the provided value is not a number.",
                  var_name = &body.variable
                ),
              ),
            )
          }
        }
        Some(VarDetail::String { max_len }) => {
          if body.value.is_text() {
            let value_str = body.value.as_str();
            let trimmed = value_str.trim();

            if trimmed.is_empty() {
              Err(
                ProblemDetail::new("Empty value", StatusCode::BAD_REQUEST)
                  .with_detail("Value cannot be empty or consist of only whitespaces.".to_owned()),
              )
            } else if trimmed.len() > *max_len {
              Err(
                ProblemDetail::new("Out of range", StatusCode::BAD_REQUEST)
                  .with_detail(format!("Maximum allowed string length is {}.", max_len)),
              )
            } else {
              Ok(())
            }
          } else {
            Err(
              ProblemDetail::new("Invalid value type", StatusCode::BAD_REQUEST).with_detail(
                format!(
                  "'{var_name}' expects a string type, but the provided value is not a string.",
                  var_name = &body.variable
                ),
              ),
            )
          }
        }
        Some(VarDetail::Enum { options }) => {
          if options.contains(&body.value) {
            Ok(())
          } else {
            Err(
              ProblemDetail::new("Invalid option", StatusCode::BAD_REQUEST).with_detail(format!(
                "'{var_name}' is an enum type, allowed options: {opts:?}",
                var_name = &body.variable,
                opts = options
                  .iter()
                  .map(|v| v.as_str())
                  .collect::<Vec<std::borrow::Cow<'_, str>>>()
              )),
            )
          }
        }
        Some(VarDetail::Range { min, max }) => {
          if body.value.is_numeric() {
            match (min.as_lossly_f64(), max.as_lossly_f64()) {
              (Some(min), Some(max)) => {
                let valuef64 = body.value.as_lossly_f64().unwrap_or(0.0);

                if min <= valuef64 && valuef64 <= max {
                  Ok(())
                } else {
                  Err(
                    ProblemDetail::new("Out of range", StatusCode::BAD_REQUEST).with_detail(
                      format!(
                        "'{var_name}' is not within the acceptable range [{min}, {max}]",
                        var_name = &body.variable,
                        min = min,
                        max = max,
                      ),
                    ),
                  )
                }
              }
              _ => Err(ProblemDetail::new("Malformed driver response", StatusCode::INTERNAL_SERVER_ERROR).with_detail(
                "Cannot process request since the reported min-max values by ups device are not number.".to_owned(),
              ),
            ),
            }
          } else {
            Err(
              ProblemDetail::new("Invalid value type", StatusCode::BAD_REQUEST).with_detail(
                format!(
                  "'{var_name}' expects a numeric value between {min} and {max}, but the provided value is not a number.",
                  var_name = &body.variable,
                  min = min,
                  max = max,
                ),
              ),
            )
          }
        }
        None => Err(
          ProblemDetail::new("Invalid RW variable", StatusCode::BAD_REQUEST).with_detail(format!(
            "'{var_name}' is not a valid writeable variable.",
            var_name = &body.variable
          )),
        ),
      },
      None => Err(ProblemDetail::new(
        "Device not found",
        StatusCode::NOT_FOUND,
      )),
    }
  }?;

  let mut client = NutAuthClient::connect(addr, user, password).await?;

  {
    let response = client.set_var(&ups_name, &body.variable, &body.value).await;
    _ = client.close().await;

    response
  }?;

  info!(
    message = "set var request accepted",
    device = %ups_name,
    variable = %body.variable,
    value = %body.value,
  );

  Ok(StatusCode::ACCEPTED)
}

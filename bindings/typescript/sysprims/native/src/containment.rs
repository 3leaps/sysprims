use napi::bindgen_prelude::{AsyncTask, BigInt, Result as NapiResult, Task};
use napi_derive::napi;
use serde::Deserialize;
use std::collections::BTreeMap;
use sysprims_core::schema::CONTAINMENT_SPAWN_CONFIG_V1;
use sysprims_core::SysprimsError;
use sysprims_timeout::{
    containment_close, containment_identity, containment_poll, containment_spawn,
    containment_terminate, containment_wait, ManagedSnapshot, ManagedSpawnRequest,
};

use super::{err_json, ok_json, SysprimsCallJsonResult};

#[napi(object)]
pub struct SysprimsCallTokenResult {
    pub code: i32,
    pub token: Option<BigInt>,
    pub message: Option<String>,
}

fn ok_token(token: u64) -> SysprimsCallTokenResult {
    SysprimsCallTokenResult {
        code: 0,
        token: Some(BigInt::from(token)),
        message: None,
    }
}

fn err_token(err: SysprimsError) -> SysprimsCallTokenResult {
    SysprimsCallTokenResult {
        code: super::SysprimsErrorCode::from(&err) as i32,
        token: None,
        message: Some(err.to_string()),
    }
}

fn token_from_bigint(token: BigInt) -> std::result::Result<u64, SysprimsError> {
    let (signed, value, lossless) = token.get_u64();
    if signed || !lossless {
        return Err(SysprimsError::invalid_argument(
            "containment handle is invalid",
        ));
    }
    Ok(value)
}

fn snapshot_json(snapshot: ManagedSnapshot) -> SysprimsCallJsonResult {
    match serde_json::to_string(&snapshot) {
        Ok(json) => ok_json(json),
        Err(error) => err_json(SysprimsError::internal(format!(
            "failed to serialize snapshot: {error}"
        ))),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSpawnConfig {
    schema_id: String,
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Option<BTreeMap<String, String>>,
    #[serde(default)]
    execution_timeout_ms: Option<u64>,
    #[serde(default)]
    grace_timeout_ms: Option<u64>,
    #[serde(default)]
    kill_timeout_ms: Option<u64>,
    #[serde(default)]
    signal: Option<i32>,
    #[serde(default)]
    kill_signal: Option<i32>,
}

pub struct ContainmentSpawnTask {
    config_json: String,
}

impl Task for ContainmentSpawnTask {
    type Output = SysprimsCallTokenResult;
    type JsValue = SysprimsCallTokenResult;

    fn compute(&mut self) -> NapiResult<Self::Output> {
        if self.config_json.is_empty() {
            return Ok(err_token(SysprimsError::invalid_argument(
                "config_json cannot be empty",
            )));
        }
        let wire = match serde_json::from_str::<WireSpawnConfig>(&self.config_json) {
            Ok(value) => value,
            Err(error) => {
                return Ok(err_token(SysprimsError::invalid_argument(format!(
                    "invalid config JSON: {error}"
                ))))
            }
        };
        if wire.schema_id != CONTAINMENT_SPAWN_CONFIG_V1 {
            return Ok(err_token(SysprimsError::invalid_argument(format!(
                "invalid schema_id (expected {CONTAINMENT_SPAWN_CONFIG_V1})"
            ))));
        }
        let request = ManagedSpawnRequest {
            argv: wire.argv,
            cwd: wire.cwd,
            env: wire.env,
            execution_timeout_ms: wire.execution_timeout_ms,
            grace_timeout_ms: wire.grace_timeout_ms,
            kill_timeout_ms: wire.kill_timeout_ms,
            signal: wire.signal,
            kill_signal: wire.kill_signal,
        };
        Ok(match containment_spawn(request) {
            Ok(token) => ok_token(token),
            Err(error) => err_token(error),
        })
    }

    fn resolve(&mut self, _env: napi::Env, output: Self::Output) -> NapiResult<Self::JsValue> {
        Ok(output)
    }
}

pub struct ContainmentJsonTask {
    token: BigInt,
    kind: JsonOp,
}

enum JsonOp {
    Identity,
    Poll,
    Wait(u64),
    Terminate,
}

impl Task for ContainmentJsonTask {
    type Output = SysprimsCallJsonResult;
    type JsValue = SysprimsCallJsonResult;

    fn compute(&mut self) -> NapiResult<Self::Output> {
        let token = match token_from_bigint(self.token.clone()) {
            Ok(token) => token,
            Err(error) => return Ok(err_json(error)),
        };
        let result = match self.kind {
            JsonOp::Identity => containment_identity(token),
            JsonOp::Poll => containment_poll(token),
            JsonOp::Wait(timeout) => containment_wait(token, timeout),
            JsonOp::Terminate => containment_terminate(token),
        };
        Ok(match result {
            Ok(snapshot) => snapshot_json(snapshot),
            Err(error) => err_json(error),
        })
    }

    fn resolve(&mut self, _env: napi::Env, output: Self::Output) -> NapiResult<Self::JsValue> {
        Ok(output)
    }
}

pub struct ContainmentCloseTask {
    token: BigInt,
}

impl Task for ContainmentCloseTask {
    type Output = super::SysprimsCallVoidResult;
    type JsValue = super::SysprimsCallVoidResult;

    fn compute(&mut self) -> NapiResult<Self::Output> {
        let token = match token_from_bigint(self.token.clone()) {
            Ok(token) => token,
            Err(error) => {
                return Ok(super::SysprimsCallVoidResult {
                    code: super::SysprimsErrorCode::from(&error) as i32,
                    message: Some(error.to_string()),
                })
            }
        };
        Ok(match containment_close(token) {
            Ok(()) => super::ok_void(),
            Err(error) => super::err_void(error),
        })
    }

    fn resolve(&mut self, _env: napi::Env, output: Self::Output) -> NapiResult<Self::JsValue> {
        Ok(output)
    }
}

#[napi]
pub fn sysprims_containment_spawn(config_json: String) -> AsyncTask<ContainmentSpawnTask> {
    AsyncTask::new(ContainmentSpawnTask { config_json })
}

#[napi]
pub fn sysprims_containment_identity(token: BigInt) -> AsyncTask<ContainmentJsonTask> {
    AsyncTask::new(ContainmentJsonTask {
        token,
        kind: JsonOp::Identity,
    })
}

#[napi]
pub fn sysprims_containment_poll(token: BigInt) -> AsyncTask<ContainmentJsonTask> {
    AsyncTask::new(ContainmentJsonTask {
        token,
        kind: JsonOp::Poll,
    })
}

#[napi]
pub fn sysprims_containment_wait(
    token: BigInt,
    wait_timeout_ms: f64,
) -> AsyncTask<ContainmentJsonTask> {
    let timeout = if wait_timeout_ms.is_finite() && wait_timeout_ms >= 0.0 {
        wait_timeout_ms.floor() as u64
    } else {
        u64::MAX
    };
    AsyncTask::new(ContainmentJsonTask {
        token,
        kind: JsonOp::Wait(timeout),
    })
}

#[napi]
pub fn sysprims_containment_terminate(token: BigInt) -> AsyncTask<ContainmentJsonTask> {
    AsyncTask::new(ContainmentJsonTask {
        token,
        kind: JsonOp::Terminate,
    })
}

#[napi]
pub fn sysprims_containment_close(token: BigInt) -> AsyncTask<ContainmentCloseTask> {
    AsyncTask::new(ContainmentCloseTask { token })
}

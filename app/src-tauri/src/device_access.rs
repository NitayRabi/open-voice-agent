//! Persistent, individually revocable credentials for browsers, phones and watches.
//! Only SHA-256 digests are written to disk; plaintext bearer tokens are returned once.

use anyhow::{Context, Result};
use parking_lot::Mutex;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Device {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub label: String,
    pub created_at: u64,
    pub last_used_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct IssuedCredential {
    pub access_token: String,
    pub token_type: &'static str,
    pub device: Device,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DeviceRecord {
    #[serde(flatten)]
    device: Device,
    token_hash: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    devices: Vec<DeviceRecord>,
}

pub struct DeviceRegistry {
    path: PathBuf,
    records: Mutex<Vec<DeviceRecord>>,
}

impl DeviceRegistry {
    pub fn load(app: &AppHandle) -> Self {
        let path = app
            .path()
            .app_config_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("paired-devices.json");
        Self::from_path(path)
    }

    fn from_path(path: PathBuf) -> Self {
        let records = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Store>(&raw).ok())
            .map(|store| store.devices)
            .unwrap_or_default();
        Self {
            path,
            records: Mutex::new(records),
        }
    }

    pub fn issue(&self, kind: &str, label: &str) -> Result<IssuedCredential> {
        let token = random_hex(32);
        let device = Device {
            id: format!("device_{}", random_hex(12)),
            kind: clean(kind, 40, "remote"),
            label: clean(label, 80, "Paired device"),
            created_at: now_ms(),
            last_used_at: None,
        };
        let mut records = self.records.lock();
        records.push(DeviceRecord {
            device: device.clone(),
            token_hash: digest(&token),
        });
        self.persist(&records)?;
        Ok(IssuedCredential {
            access_token: token,
            token_type: "Bearer",
            device,
        })
    }

    pub fn authenticate(&self, token: &str) -> Option<String> {
        let hash = digest(token.trim());
        let mut records = self.records.lock();
        let record = records.iter_mut().find(|r| r.token_hash == hash)?;
        let now = now_ms();
        let should_write = record
            .device
            .last_used_at
            .map_or(true, |last| now.saturating_sub(last) >= 3_600_000);
        record.device.last_used_at = Some(now);
        let id = record.device.id.clone();
        if should_write {
            let _ = self.persist(&records);
        }
        Some(id)
    }

    pub fn is_valid_for(&self, token: &str, device_id: &str) -> bool {
        let hash = digest(token.trim());
        self.records
            .lock()
            .iter()
            .any(|r| r.device.id == device_id && r.token_hash == hash)
    }

    pub fn list(&self) -> Vec<Device> {
        self.records
            .lock()
            .iter()
            .map(|r| r.device.clone())
            .collect()
    }

    pub fn revoke(&self, id: &str) -> Result<bool> {
        let mut records = self.records.lock();
        let old_len = records.len();
        records.retain(|r| r.device.id != id);
        if records.len() == old_len {
            return Ok(false);
        }
        self.persist(&records)?;
        Ok(true)
    }

    fn persist(&self, records: &[DeviceRecord]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(&Store {
                devices: records.to_vec(),
            })?,
        )
        .with_context(|| format!("writing {tmp:?}"))?;
        fs::rename(&tmp, &self.path).with_context(|| format!("replacing {:?}", self.path))?;
        Ok(())
    }

    #[cfg(test)]
    fn test_at(path: PathBuf) -> Self {
        Self::from_path(path)
    }
}

fn random_hex(bytes: usize) -> String {
    let mut value = vec![0_u8; bytes];
    rand::thread_rng().fill_bytes(&mut value);
    hex::encode(value)
}
fn clean(value: &str, max: usize, fallback: &str) -> String {
    let value: String = value.trim().chars().take(max).collect();
    if value.is_empty() {
        fallback.into()
    } else {
        value
    }
}
fn digest(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::DeviceRegistry;
    fn path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ova-device-test-{name}-{}", std::process::id()))
    }

    #[test]
    fn tokens_are_only_persisted_as_hashes() {
        let path = path("persist");
        let _ = std::fs::remove_file(&path);
        let registry = DeviceRegistry::test_at(path.clone());
        let issued = registry.issue("wear", "Pixel Watch").unwrap();
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains(&issued.access_token));
        assert_eq!(
            DeviceRegistry::test_at(path.clone()).authenticate(&issued.access_token),
            Some(issued.device.id)
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn revocation_is_per_device() {
        let path = path("revoke");
        let _ = std::fs::remove_file(&path);
        let registry = DeviceRegistry::test_at(path.clone());
        let watch = registry.issue("wear", "Watch").unwrap();
        let phone = registry.issue("android", "Phone").unwrap();
        assert!(registry.revoke(&watch.device.id).unwrap());
        assert_eq!(registry.authenticate(&watch.access_token), None);
        assert!(registry.authenticate(&phone.access_token).is_some());
        assert!(!registry.revoke("missing").unwrap());
        let _ = std::fs::remove_file(path);
    }
}

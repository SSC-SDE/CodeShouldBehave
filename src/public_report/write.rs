use super::{PublicBundle, canonical_json};
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path},
};

// Parse every object with duplicate rejection before constructing Value. A later
// typed conversion cannot detect keys already collapsed by Value's map parser.
struct UniqueJson(Value);
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("bounded JSON without duplicate object keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Bool(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Number(v.into())))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Number(v.into())))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| UniqueJson(Value::Number(n)))
                    .ok_or_else(|| E::custom("Nonfinite JSON number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::String(v.into())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::String(v)))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = vec![];
                while let Some(UniqueJson(value)) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(UniqueJson(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom("Duplicate JSON key"));
                    }
                    let UniqueJson(value) = map.next_value()?;
                    values.insert(key, value);
                }
                Ok(UniqueJson(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}
pub(super) fn parse_json(bytes: &[u8]) -> Result<Value, String> {
    serde_json::from_slice::<UniqueJson>(bytes)
        .map(|v| v.0)
        .map_err(|_| "Invalid or duplicate-key input JSON".into())
}

/// Operator paths must already have trusted non-symlink ancestors. No arbitrary
/// path from private JSON is used for filesystem output.
fn no_symlink(path: &Path) -> Result<(), String> {
    let mut prefix = std::path::PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return Err("Parent paths are not permitted".into());
        }
        prefix.push(component);
        match fs::symlink_metadata(&prefix) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("Symlink operator path refused".into());
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Operator path metadata unavailable".into()),
        }
    }
    Ok(())
}
pub fn read_json_file(path: &Path, limit: usize) -> Result<Value, String> {
    no_symlink(path)?;
    let meta = fs::symlink_metadata(path).map_err(|_| "Input unavailable")?;
    if !meta.is_file() || meta.len() > limit as u64 {
        return Err("Input is not a bounded regular file".into());
    }
    let mut bytes = vec![];
    fs::File::open(path)
        .map_err(|_| "Input unavailable")?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Input read failed")?;
    if bytes.len() > limit {
        return Err("Input byte limit".into());
    }
    parse_json(&bytes)
}
pub fn write_bundle(bundle: &PublicBundle, output: &Path) -> Result<(), String> {
    bundle.validate()?;
    no_symlink(output)?;
    fs::create_dir(output).map_err(|_| "Output must be a new directory under a trusted parent")?;
    let result = (|| {
        for (path, bytes) in &bundle.files {
            let target = output.join(path);
            fs::create_dir_all(target.parent().ok_or("Invalid output parent")?)
                .map_err(|_| "Cannot create output parent")?;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .and_then(|mut file| file.write_all(bytes))
                .map_err(|_| "Cannot create public chunk")?;
        }
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join("manifest.json"))
            .and_then(|mut file| {
                file.write_all(&canonical_json(&bundle.manifest).map_err(std::io::Error::other)?)
            })
            .map_err(|_| "Cannot create public manifest")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(output);
    }
    result
}

/// Integrity validation only. The publisher separately verifies trusted producer,
/// expected repository/commits/policy, current public consent and provider state.
pub fn verify_bundle(directory: &Path) -> Result<super::PublicManifest, String> {
    no_symlink(directory)?;
    if !fs::symlink_metadata(directory)
        .map_err(|_| "Bundle unavailable")?
        .is_dir()
    {
        return Err("Bundle must be a regular directory".into());
    }
    let manifest_path = directory.join("manifest.json");
    let manifest: super::PublicManifest =
        serde_json::from_value(read_json_file(&manifest_path, super::MAX_PUBLIC_BYTES)?)
            .map_err(|_| "Invalid strict public manifest")?;
    if manifest.files.len() >= super::MAX_PUBLIC_FILES {
        return Err("Public file count exceeds bound".into());
    }
    let mut pending = vec![directory.to_path_buf()];
    let mut actual = std::collections::BTreeSet::new();
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).map_err(|_| "Bundle inventory unavailable")? {
            let entry = entry.map_err(|_| "Bundle inventory unavailable")?;
            let meta =
                fs::symlink_metadata(entry.path()).map_err(|_| "Bundle metadata unavailable")?;
            let relative = entry
                .path()
                .strip_prefix(directory)
                .map_err(|_| "Bundle prefix mismatch")?
                .to_str()
                .ok_or("Non-UTF8 bundle path")?
                .to_string();
            super::policy::safe_path(&relative)?;
            if meta.file_type().is_symlink() {
                return Err("Symlink public bundle entry".into());
            }
            if meta.is_dir() {
                if !manifest
                    .files
                    .iter()
                    .any(|f| f.path.starts_with(&(relative.clone() + "/")))
                {
                    return Err("Unexpected public bundle directory".into());
                }
                if relative.split('/').count() > 4 || pending.len() > super::MAX_PUBLIC_FILES {
                    return Err("Unbounded public directory tree".into());
                }
                pending.push(entry.path());
            } else if meta.is_file() {
                actual.insert(relative);
                if actual.len() > super::MAX_PUBLIC_FILES {
                    return Err("Public file count exceeds bound".into());
                }
            } else {
                return Err("Non-regular public bundle entry".into());
            }
        }
    }
    let expected = manifest
        .files
        .iter()
        .map(|f| f.path.clone())
        .chain(std::iter::once("manifest.json".into()))
        .collect();
    if actual != expected {
        return Err("Unexpected or missing public bundle file".into());
    }
    let mut files = std::collections::BTreeMap::new();
    let mut total = fs::symlink_metadata(&manifest_path)
        .map_err(|_| "Manifest metadata unavailable")?
        .len() as usize;
    for file in &manifest.files {
        super::policy::safe_path(&file.path)?;
        if file.bytes > super::MAX_PUBLIC_BYTES {
            return Err("Public file exceeds byte bound".into());
        }
        let mut bytes = vec![];
        fs::File::open(directory.join(&file.path))
            .map_err(|_| "Public chunk unavailable")?
            .take(file.bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Public chunk read failed")?;
        total = total.saturating_add(bytes.len());
        if total > super::MAX_PUBLIC_BYTES {
            return Err("Public bundle byte limit".into());
        }
        files.insert(file.path.clone(), bytes);
    }
    let bundle = PublicBundle { manifest, files };
    bundle.validate()?;
    Ok(bundle.manifest)
}

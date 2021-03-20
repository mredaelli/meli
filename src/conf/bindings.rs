/*
 * meli - configuration module.
 *
 * Copyright 2019 Manos Pitsidianakis
 *
 * This file is part of meli.
 *
 * meli is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * meli is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with meli. If not, see <http://www.gnu.org/licenses/>.
 */

use crate::terminal::Key;
use melib::{MeliError, Result};
use serde::de::{Deserialize, Deserializer};
use std::collections::HashMap;
use toml::Value;

type BindingType = HashMap<Vec<Key>, String>;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bindings {
    #[serde(deserialize_with = "deserialize_bindings")]
    pub normal: BindingType,
}

pub fn filter(bs: &BindingType, key: &[(Key, Vec<u8>)]) -> BindingType {
    let mut res: BindingType = bs.clone();
    for (idx, (k, _)) in key.iter().enumerate() {
        res = res.into_iter()
            .filter(|(ks, _)| ks.get(idx).map(|c| c == k).unwrap_or(false))
            .collect()
    }
    res
}

fn string_to_keys(s: &str) -> Result<Vec<Key>> {
    s.split_whitespace()
        .map(|s| {
            Key::deserialize(Value::String(s.to_owned()))
                .map_err(|e| MeliError::new(format!("{}", e)))
        })
        .collect()
}

fn deserialize_bindings<'de, D>(deserializer: D) -> std::result::Result<BindingType, D::Error>
where
    D: Deserializer<'de>,
{
    let map: HashMap<String, String> = HashMap::deserialize(deserializer)?;
    let (ok, errors): (Vec<_>, Vec<_>) = map
        .into_iter()
        .map(|(k, v)| (string_to_keys(&k), v))
        .partition(|(k, _)| k.is_ok());

    if !errors.is_empty() {
        Err(serde::de::Error::custom(
            errors
                .into_iter()
                .map(|(k, _)| format!("{}", k.err().unwrap()))
                .collect::<Vec<_>>()
                .join(","),
        ))
    } else {
        Ok(ok.into_iter().map(|(k, v)| (k.unwrap(), v)).collect())
    }
}

impl Default for Bindings {
    fn default() -> Self {
        Self {
            normal: HashMap::new(),
        }
    }
}

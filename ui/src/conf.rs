/*
 * meli - configuration module.
 *
 * Copyright 2017 Manos Pitsidianakis
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

extern crate bincode;
extern crate serde;
extern crate toml;
extern crate xdg;

pub mod composing;
pub mod notifications;
pub mod pager;
pub mod pgp;
pub mod shortcuts;
pub mod terminal;

pub mod accounts;
pub use self::accounts::Account;
pub use self::composing::*;
pub use self::pgp::*;
pub use self::shortcuts::*;

use self::default_vals::*;
use self::notifications::NotificationsSettings;
use self::terminal::TerminalSettings;
use crate::pager::PagerSettings;
use melib::backends::SpecialUseMailbox;
use melib::conf::{toggleflag_de, AccountSettings, FolderConf, ToggleFlag};
use melib::error::*;

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use std::collections::HashMap;
use std::env;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[macro_export]
macro_rules! split_command {
    ($cmd:expr) => {{
        $cmd.split_whitespace().collect::<Vec<&str>>()
    }};
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct MailUIConf {
    pub pager: Option<PagerSettings>,
    pub notifications: Option<NotificationsSettings>,
    pub shortcuts: Option<Shortcuts>,
    pub composing: Option<ComposingSettings>,
    pub identity: Option<String>,
    pub index_style: Option<IndexStyle>,
}

#[serde(default)]
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct FileFolderConf {
    #[serde(flatten)]
    pub folder_conf: FolderConf,
    #[serde(flatten)]
    pub conf_override: MailUIConf,
}

impl FileFolderConf {
    pub fn conf_override(&self) -> &MailUIConf {
        &self.conf_override
    }

    pub fn folder_conf(&self) -> &FolderConf {
        &self.folder_conf
    }
}

use crate::conf::deserializers::extra_settings;
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileAccount {
    root_folder: String,
    format: String,
    identity: String,
    #[serde(flatten)]
    #[serde(deserialize_with = "extra_settings")]
    pub extra: HashMap<String, String>, /* use custom deserializer to convert any given value (eg bool, number, etc) to string */

    #[serde(default = "none")]
    display_name: Option<String>,
    index_style: IndexStyle,

    #[serde(default = "false_val")]
    read_only: bool,
    subscribed_folders: Vec<String>,
    #[serde(default)]
    folders: HashMap<String, FileFolderConf>,
    #[serde(default)]
    cache_type: CacheType,
}

impl From<FileAccount> for AccountConf {
    fn from(x: FileAccount) -> Self {
        let format = x.format.to_lowercase();
        let root_folder = x.root_folder.clone();
        let identity = x.identity.clone();
        let display_name = x.display_name.clone();
        let folders = x
            .folders
            .iter()
            .map(|(k, v)| (k.clone(), v.folder_conf.clone()))
            .collect();

        let mut acc = AccountSettings {
            name: String::new(),
            root_folder,
            format,
            identity,
            read_only: x.read_only,
            display_name,
            subscribed_folders: x.subscribed_folders.clone(),
            folders,
            extra: x.extra.clone(),
        };

        let root_path = PathBuf::from(acc.root_folder.as_str());
        let root_tmp = root_path
            .components()
            .last()
            .unwrap()
            .as_os_str()
            .to_str()
            .unwrap()
            .to_string();
        if !acc.subscribed_folders.contains(&root_tmp) {
            acc.subscribed_folders.push(root_tmp);
        }
        let mut folder_confs = x.folders.clone();
        for s in &x.subscribed_folders {
            if !folder_confs.contains_key(s) {
                use text_processing::GlobMatch;
                if s.is_glob() {
                    continue;
                }
                folder_confs.insert(
                    s.to_string(),
                    FileFolderConf {
                        folder_conf: FolderConf {
                            subscribe: ToggleFlag::True,
                            ..FolderConf::default()
                        },
                        ..FileFolderConf::default()
                    },
                );
            } else {
                if !folder_confs[s].folder_conf().subscribe.is_unset() {
                    continue;
                }
                folder_confs.get_mut(s).unwrap().folder_conf.subscribe = ToggleFlag::True;
            }

            if folder_confs[s].folder_conf().usage.is_none() {
                let name = s
                    .split(if s.contains('/') { '/' } else { '.' })
                    .last()
                    .unwrap_or("");
                folder_confs.get_mut(s).unwrap().folder_conf.usage = usage(name);
            }

            if folder_confs[s].folder_conf().ignore.is_unset() {
                use SpecialUseMailbox::*;
                if [Junk, Sent, Trash]
                    .contains(&folder_confs[s].folder_conf().usage.as_ref().unwrap())
                {
                    folder_confs.get_mut(s).unwrap().folder_conf.ignore =
                        ToggleFlag::InternalVal(true);
                }
            }
        }

        AccountConf {
            account: acc,
            conf: x,
            folder_confs,
        }
    }
}

impl FileAccount {
    pub fn folders(&self) -> &HashMap<String, FileFolderConf> {
        &self.folders
    }

    pub fn folder(&self) -> &str {
        &self.root_folder
    }

    pub fn index_style(&self) -> IndexStyle {
        self.index_style
    }

    pub fn cache_type(&self) -> &CacheType {
        &self.cache_type
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileSettings {
    accounts: HashMap<String, FileAccount>,
    #[serde(default)]
    pager: PagerSettings,
    #[serde(default)]
    notifications: NotificationsSettings,
    #[serde(default)]
    shortcuts: Shortcuts,
    composing: ComposingSettings,
    #[serde(default)]
    pgp: PGPSettings,
    #[serde(default)]
    terminal: TerminalSettings,
}

#[derive(Debug, Clone, Default)]
pub struct AccountConf {
    pub(crate) account: AccountSettings,
    pub(crate) conf: FileAccount,
    pub(crate) folder_confs: HashMap<String, FileFolderConf>,
}

impl AccountConf {
    pub fn account(&self) -> &AccountSettings {
        &self.account
    }
    pub fn conf(&self) -> &FileAccount {
        &self.conf
    }
    pub fn conf_mut(&mut self) -> &mut FileAccount {
        &mut self.conf
    }
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub accounts: HashMap<String, AccountConf>,
    pub pager: PagerSettings,
    pub notifications: NotificationsSettings,
    pub shortcuts: Shortcuts,
    pub composing: ComposingSettings,
    pub pgp: PGPSettings,
    pub terminal: TerminalSettings,
}

impl FileSettings {
    pub fn new() -> Result<FileSettings> {
        let config_path = match env::var("MELI_CONFIG") {
            Ok(path) => PathBuf::from(path),
            Err(_) => {
                let xdg_dirs = xdg::BaseDirectories::with_prefix("meli").unwrap();
                xdg_dirs
                    .place_config_file("config")
                    .expect("cannot create configuration directory")
            }
        };
        if !config_path.exists() {
            println!(
                "No configuration found. Would you like to generate one in {}? [Y/n]",
                config_path.display()
            );
            let mut buffer = String::new();
            let stdin = io::stdin();
            let mut handle = stdin.lock();

            loop {
                buffer.clear();
                handle
                    .read_line(&mut buffer)
                    .expect("Could not read from stdin.");

                match buffer.trim() {
                    "Y" | "y" | "yes" | "YES" | "Yes" => {
                        create_config_file(&config_path)?;
                        return Err(MeliError::new(
                            "Edit the sample configuration and relaunch meli.",
                        ));
                    }
                    "n" | "N" | "no" | "No" | "NO" => {
                        return Err(MeliError::new("No configuration file found."));
                    }
                    _ => {
                        println!(
                            "No configuration found. Would you like to generate one in {}? [Y/n]",
                            config_path.display()
                        );
                    }
                }
            }
        }

        let mut file = File::open(config_path.to_str().unwrap())?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        let s = toml::from_str(&contents);
        if let Err(e) = s {
            return Err(MeliError::new(format!(
                "Config file contains errors: {}",
                e.to_string()
            )));
        }

        Ok(s.unwrap())
    }

    pub fn validate(path: &str) -> Result<()> {
        let mut file = File::open(path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        let s: std::result::Result<FileSettings, toml::de::Error> = toml::from_str(&contents);
        if let Err(e) = s {
            return Err(MeliError::new(format!(
                "Config file contains errors: {}",
                e.to_string()
            )));
        }

        Ok(())
    }
}

impl Settings {
    pub fn new() -> Result<Settings> {
        let fs = FileSettings::new()?;
        let mut s: HashMap<String, AccountConf> = HashMap::new();

        for (id, x) in fs.accounts {
            let mut ac = AccountConf::from(x);
            ac.account.set_name(id.clone());

            s.insert(id, ac);
        }

        Ok(Settings {
            accounts: s,
            pager: fs.pager,
            notifications: fs.notifications,
            shortcuts: fs.shortcuts,
            composing: fs.composing,
            pgp: fs.pgp,
            terminal: fs.terminal,
        })
    }
}

#[derive(Copy, Debug, Clone, Hash, PartialEq)]
pub enum IndexStyle {
    Plain,
    Threaded,
    Compact,
    Conversations,
}

impl Default for IndexStyle {
    fn default() -> Self {
        IndexStyle::Compact
    }
}

/*
 * Deserialize default functions
 */

mod default_vals {
    pub(in crate::conf) fn false_val() -> bool {
        false
    }

    pub(in crate::conf) fn true_val() -> bool {
        true
    }

    pub(in crate::conf) fn zero_val() -> usize {
        0
    }

    pub(in crate::conf) fn eighty_percent() -> usize {
        80
    }

    pub(in crate::conf) fn none<T>() -> Option<T> {
        None
    }

    pub(in crate::conf) fn internal_value_false() -> super::ToggleFlag {
        super::ToggleFlag::InternalVal(false)
    }
}

mod deserializers {
    use serde::{Deserialize, Deserializer};
    pub(in crate::conf) fn non_empty_string<'de, D>(
        deserializer: D,
    ) -> std::result::Result<Option<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <String>::deserialize(deserializer)?;
        if s.is_empty() {
            Ok(None)
        } else {
            Ok(Some(s))
        }
    }

    use toml::Value;
    fn any_of<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v: Value = Deserialize::deserialize(deserializer)?;
        let mut ret = v.to_string();
        if ret.starts_with('"') && ret.ends_with('"') {
            ret.drain(0..1).count();
            ret.drain(ret.len() - 1..).count();
        }
        Ok(ret)
    }

    use std::collections::HashMap;
    pub(in crate::conf) fn extra_settings<'de, D>(
        deserializer: D,
    ) -> std::result::Result<HashMap<String, String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        /* Why is this needed? If the user gives a configuration value such as key = true, the
         * parsing will fail since it expects string values. We want to accept key = true as well
         * as key = "true". */
        #[derive(Deserialize)]
        struct Wrapper(#[serde(deserialize_with = "any_of")] String);

        let v = <HashMap<String, Wrapper>>::deserialize(deserializer)?;
        Ok(v.into_iter().map(|(k, Wrapper(v))| (k, v)).collect())
    }
}

impl<'de> Deserialize<'de> for IndexStyle {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <String>::deserialize(deserializer)?;
        match s.as_str() {
            "Plain" | "plain" => Ok(IndexStyle::Plain),
            "Threaded" | "threaded" => Ok(IndexStyle::Threaded),
            "Compact" | "compact" => Ok(IndexStyle::Compact),
            "Conversations" | "conversations" => Ok(IndexStyle::Conversations),
            _ => Err(de::Error::custom("invalid `index_style` value")),
        }
    }
}

impl Serialize for IndexStyle {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            IndexStyle::Plain => serializer.serialize_str("plain"),
            IndexStyle::Threaded => serializer.serialize_str("threaded"),
            IndexStyle::Compact => serializer.serialize_str("compact"),
            IndexStyle::Conversations => serializer.serialize_str("conversations"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CacheType {
    None,
    #[cfg(feature = "sqlite3")]
    Sqlite3,
}

impl Default for CacheType {
    fn default() -> Self {
        #[cfg(feature = "sqlite3")]
        {
            CacheType::Sqlite3
        }
        #[cfg(not(feature = "sqlite3"))]
        {
            CacheType::None
        }
    }
}

impl<'de> Deserialize<'de> for CacheType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <String>::deserialize(deserializer)?;
        match s.as_str() {
            #[cfg(feature = "sqlite3")]
            "sqlite3" => Ok(CacheType::Sqlite3),
            "nothing" | "none" | "" => Ok(CacheType::None),
            _ => Err(de::Error::custom("invalid `index_cache` value")),
        }
    }
}

impl Serialize for CacheType {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            #[cfg(feature = "sqlite3")]
            CacheType::Sqlite3 => serializer.serialize_str("sqlite3"),
            CacheType::None => serializer.serialize_str("none"),
        }
    }
}

pub fn usage(name: &str) -> Option<SpecialUseMailbox> {
    if name.eq_ignore_ascii_case("inbox") {
        Some(SpecialUseMailbox::Inbox)
    } else if name.eq_ignore_ascii_case("archive") {
        Some(SpecialUseMailbox::Archive)
    } else if name.eq_ignore_ascii_case("drafts") {
        Some(SpecialUseMailbox::Drafts)
    } else if name.eq_ignore_ascii_case("junk") {
        Some(SpecialUseMailbox::Junk)
    } else if name.eq_ignore_ascii_case("spam") {
        Some(SpecialUseMailbox::Junk)
    } else if name.eq_ignore_ascii_case("sent") {
        Some(SpecialUseMailbox::Sent)
    } else if name.eq_ignore_ascii_case("trash") {
        Some(SpecialUseMailbox::Trash)
    } else {
        Some(SpecialUseMailbox::Normal)
    }
}

pub fn create_config_file(p: &Path) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(p)
        .expect("Could not create config file.");
    file.write_all(include_bytes!("../../sample-config"))
        .expect("Could not write to config file.");
    println!("Written example configuration to {}", p.display());
    let metadata = file.metadata()?;
    let mut permissions = metadata.permissions();

    permissions.set_mode(0o600); // Read/write for owner only.
    file.set_permissions(permissions)?;
    Ok(())
}
